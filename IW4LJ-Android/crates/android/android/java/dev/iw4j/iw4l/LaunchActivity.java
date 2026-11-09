package dev.iw4j.iw4l;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.IOException;
import java.io.OutputStream;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class LaunchActivity extends Activity {
    private static final int GAME = 1, EXPORT = 2;
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private File attempt;
    private TextView body;
    private Button start;
    private String report = "";
    private int recycles;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        if (getIntent().getBooleanExtra("iw4l_dev", false)) {
            setShowWhenLocked(true);
            setTurnScreenOn(true);
        }
        attempt = StartupReport.latest(this);
        StartupReport.boundAttempt = attempt;
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        int pad = (int) (16 * getResources().getDisplayMetrics().density);
        page.setPadding(pad, pad, pad, pad);
        TextView heading = new TextView(this);
        heading.setText("IW4LJ Thor Test");
        heading.setTextSize(22);
        page.addView(heading);
        body = new TextView(this);
        body.setTextIsSelectable(true);
        body.setText("Opening setup and diagnostics…");
        ScrollView scroll = new ScrollView(this);
        scroll.addView(body);
        page.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        LinearLayout actions = new LinearLayout(this);
        start = addButton(actions, attempt == null ? "Start game" : "Start / retry", v -> begin());
        addButton(actions, "Copy", v -> {
            ((ClipboardManager) getSystemService(CLIPBOARD_SERVICE))
                .setPrimaryClip(ClipData.newPlainText("IW4LJ report", report));
            toast("Copied startup summary");
        });
        addButton(actions, "Share", v -> {
            Intent intent = new Intent(Intent.ACTION_SEND).setType("text/plain")
                .putExtra(Intent.EXTRA_TEXT, report);
            startActivity(Intent.createChooser(intent, "Share startup summary"));
        });
        addButton(actions, "Export ZIP", v -> {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT)
                .addCategory(Intent.CATEGORY_OPENABLE).setType("application/zip")
                .putExtra(Intent.EXTRA_TITLE, "IW4LJ-startup-" + System.currentTimeMillis() + ".zip");
            startActivityForResult(intent, EXPORT);
        });
        page.addView(actions);
        setContentView(page);
    }

    @Override
    protected void onResume() {
        super.onResume();
        bindLatest();
        refresh();
    }

    private void bindLatest() {
        attempt = StartupReport.latest(this);
        StartupReport.boundAttempt = attempt;
    }

    private void refresh() {
        final File dir = attempt;
        worker.submit(() -> {
            update(dir, StartupReport.summary(this, dir));
            StartupReport.collect(this, dir);
            update(dir, StartupReport.summary(this, dir));
        });
    }

    private void update(File dir, String text) {
        runOnUiThread(() -> {
            if (isDestroyed() || isFinishing() || !java.util.Objects.equals(attempt, dir)) return;
            report = text;
            body.setText(text);
        });
    }

    private void begin() {
        start.setEnabled(false);
        recycles = 0;
        try {
            attempt = StartupReport.begin(this);
            launchEngine();
        } catch (IOException | RuntimeException error) {
            start.setEnabled(true);
            report = "Could not start an attempt: " + StartupReport.stack(error);
            body.setText(report);
            StartupReport.append(getFilesDir(), "java-error.txt", report);
        }
    }

    private void launchEngine() {
        Intent loader = new Intent(this, EngineLoaderActivity.class)
            .putExtra(StartupReport.EXTRA, attempt.getName())
            .putExtra("iw4l_dev", getIntent().getBooleanExtra("iw4l_dev", false));
        StartupReport.event(attempt, "dispatching private ARM64 engine activity");
        startActivityForResult(loader, GAME);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == GAME) {
            bindLatest();
            if (result == EngineLoaderActivity.RECYCLE && recycles++ < 2 && attempt != null) {
                start.postDelayed(() -> {
                    if (!isFinishing() && !isDestroyed()) launchEngine();
                }, 250);
                return;
            }
            StartupReport.event(attempt, "engine activity returned (result " + result + ")");
            start.setEnabled(true);
            start.setText("Start / retry");
            refresh();
        } else if (request == EXPORT && result == RESULT_OK && data != null && data.getData() != null) {
            Uri uri = data.getData();
            File dir = attempt;
            worker.submit(() -> {
                try {
                    StartupReport.collect(this, dir);
                    try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                        if (out == null) throw new IOException("Document provider returned no stream");
                        StartupReport.export(this, out);
                    }
                    runOnUiThread(() -> toast("Startup ZIP saved"));
                } catch (Exception error) {
                    runOnUiThread(() -> toast("Export failed: " + error));
                }
            });
        }
    }

    @Override
    protected void onDestroy() {
        worker.shutdown();
        super.onDestroy();
    }

    private Button addButton(LinearLayout row, String title, View.OnClickListener action) {
        Button button = new Button(this);
        button.setText(title);
        button.setOnClickListener(action);
        row.addView(button, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        return button;
    }

    private void toast(String text) {
        Toast.makeText(this, text, Toast.LENGTH_LONG).show();
    }
}
