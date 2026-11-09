#!/usr/bin/env bash
set -euo pipefail

qa_out="$RUNNER_TEMP/thor-qa"
mkdir -p "$qa_out"
echo "QA_OUT=$qa_out" >> "$GITHUB_ENV"
api="repos/$GITHUB_REPOSITORY/actions"
same_inputs() {
    git merge-base --is-ancestor "$1" HEAD &&
        git diff --quiet "$1" HEAD -- IW4LJ-Android/Cargo.toml IW4LJ-Android/Cargo.lock \
            IW4LJ-Android/.cargo IW4LJ-Android/crates IW4LJ-Android/vendor IW4LJ-Android/xtask \
            .github/workflows/thor-android.yml \
            ':!IW4LJ-Android/crates/approved_tests/android_startup_fixture.py'
}
expected_sha=""
build_run=""
if [[ "$GITHUB_WORKFLOW" == "Thor Android build" ]]; then
    build_run=$GITHUB_RUN_ID
elif [[ -n "${BUILD_RUN:-}" ]]; then
    [[ "$BUILD_RUN" =~ ^[0-9]+$ ]] || { echo "::error::Invalid build run ID"; exit 1; }
    build_run=$BUILD_RUN
elif [[ -f .github/thor-release.json ]]; then
    receipt_sha=$(jq -er '.source_sha | select(test("^[a-f0-9]{40}$"))' .github/thor-release.json)
    if same_inputs "$receipt_sha"; then
        expected_sha=$receipt_sha
        build_run=$(jq -er '.run_id | numbers' .github/thor-release.json)
    fi
fi
if [[ -z "$build_run" ]]; then
    # A newer native change invalidates the last release receipt. Wait for
    # the Actions index if this push triggered a matching build in parallel.
    index_deadline=$((SECONDS + 120))
    while [[ -z "$build_run" ]] && (( SECONDS < index_deadline )); do
        runs=$(gh api "$api/runs?branch=repair%2Fthor-startup&per_page=50")
        while read -r id source; do
            if same_inputs "$source"; then
                build_run=$id
                break
            fi
        done < <(jq -r '.workflow_runs[] | select(.path == ".github/workflows/thor-android.yml") | select(.status != "completed" or .conclusion == "success" or .conclusion == "failure") | [.id,.head_sha] | @tsv' <<< "$runs")
        [[ -n "$build_run" ]] || sleep 10
    done
    [[ -n "$build_run" ]] || { echo "::error::No branch build matches these sources"; exit 1; }
fi
run=$(gh api "$api/runs/$build_run")
jq -e '.head_branch == "repair/thor-startup" and .path == ".github/workflows/thor-android.yml"' <<< "$run" >/dev/null
if [[ -n "$expected_sha" ]]; then
    jq -e --arg sha "$expected_sha" '.head_sha == $sha' <<< "$run" >/dev/null
fi
jq '{id,html_url,head_sha,head_branch,path}' <<< "$run" > "$qa_out/build.json"
echo "Testing $(jq -r .html_url <<< "$run") at $(jq -r .head_sha <<< "$run")"
deadline=$((SECONDS + 45 * 60))
until gh api "$api/runs/$build_run/artifacts" --jq '.artifacts[] | select(.name == "thor-build" and .expired == false) | .id' | grep -q '[0-9]'; do
    if [[ "$(gh api "$api/runs/$build_run" --jq .status)" == completed ]]; then
        echo "::error::Completed build has no unexpired thor-build artifact"
        exit 1
    fi
    if (( SECONDS >= deadline )); then
        echo "::error::Timed out waiting 45 minutes for the selected build artifact"
        exit 1
    fi
    sleep 30
done
gh run download "$build_run" --name thor-build --dir "$RUNNER_TEMP/thor-build"
(cd "$RUNNER_TEMP/thor-build" && sha256sum -c SHA256SUMS-unsigned)
