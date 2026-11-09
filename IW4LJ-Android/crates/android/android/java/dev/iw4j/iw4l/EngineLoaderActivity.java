package dev.iw4j.iw4l;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.system.Os;
import android.widget.TextView;

import java.io.File;
import java.io.IOException;
import java.util.LinkedHashSet;

public final class EngineLoaderActivity extends Activity {
    static final int RECYCLE = RESULT_FIRST_USER;
    private static volatile String processAttempt;
    private static LinkedHashSet<File> loadedRoots;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        File dir = attemptFor(this);
        TextView status = new TextView(this);
        status.setGravity(android.view.Gravity.CENTER);
        status.setText("Loading the ARM64 game library…");
        setContentView(status);
        if (processAttempt != null && !processAttempt.equals(dir.getName())) {
            // winit and the Rust task pools are process singletons. Recycle only
            // this idle engine process between explicit attempts, before JNI.
            StartupReport.event(dir, "restarting an earlier engine process");
            setResult(RECYCLE);
            finish();
            android.os.Process.killProcess(android.os.Process.myPid());
            return;
        }
        processAttempt = dir.getName();
        new Thread(() -> {
            try {
                if (!hasGameDirectory(dir, prepare(this, dir))) {
                    runOnUiThread(this::finish);
                    return;
                }
                runOnUiThread(() -> {
                    if (isFinishing() || isDestroyed()) return;
                    Intent game = new Intent(this, GameActivity.class)
                        .putExtra(StartupReport.EXTRA, dir.getName())
                        .putExtra("iw4l_dev", getIntent().getBooleanExtra("iw4l_dev", false))
                        .addFlags(Intent.FLAG_ACTIVITY_FORWARD_RESULT);
                    StartupReport.event(dir, "launching NativeActivity");
                    startActivity(game);
                    finish();
                });
            } catch (Exception | LinkageError problem) {
                StartupReport.append(dir, "java-error.txt",
                    "Engine loader: " + StartupReport.stack(problem));
                runOnUiThread(this::finish);
            }
        }, "iw4l-loader").start();
    }

    static File attemptFor(Activity activity) {
        File dir = StartupReport.from(activity.getIntent(), activity);
        if (processAttempt == null && StartupReport.present(dir, "engine.pid")) {
            File interrupted = dir;
            try {
                dir = StartupReport.begin(activity);
            } catch (IOException error) {
                throw new IllegalStateException("Could not create a fresh report for the restored activity", error);
            }
            activity.getIntent().putExtra(StartupReport.EXTRA, dir.getName());
            StartupReport.event(dir, "restored after process death; earlier attempt " + interrupted.getName());
            new Thread(() -> StartupReport.collect(activity.getApplicationContext(), interrupted),
                "iw4l-exit-history").start();
        }
        StartupReport.boundAttempt = dir;
        return dir;
    }

    private static LinkedHashSet<File> gameRoots(File media, File files, File[] mediaDirs,
                                                  File[] fileDirs, String override) {
        LinkedHashSet<File> roots = new LinkedHashSet<>();
        roots.add(new File(media, "mw2"));
        roots.add(new File(files, "mw2"));
        for (int i = 0; i < Math.max(mediaDirs.length, fileDirs.length); i++) {
            if (i < mediaDirs.length && mediaDirs[i] != null) roots.add(new File(mediaDirs[i], "mw2"));
            if (i < fileDirs.length && fileDirs[i] != null) roots.add(new File(fileDirs[i], "mw2"));
        }
        for (File base : mediaDirs) {
            if (base == null || base.getParentFile() == null) continue;
            File original = new File(base.getParentFile(), "dev.iw4j.iw4l");
            if (original.isDirectory() && original.canRead()) roots.add(new File(original, "mw2"));
        }
        if (override != null && !override.isEmpty()) roots.add(new File(override).getAbsoluteFile());
        return roots;
    }

    private static boolean hasGameDirectory(File dir, LinkedHashSet<File> candidates) throws IOException {
        String settings = Os.getenv("IW4L_SETTINGS_PATH");
        File config = settings == null
            ? new File(Os.getenv("XDG_CONFIG_HOME"), "iw4l/settings.cfg") : new File(settings);
        // Additional user-selected roots are decoded by the engine.
        if (config.isFile()) return true;
        StringBuilder error = new StringBuilder("No accessible MW2 multiplayer directories. Looked for:\n");
        for (File root : candidates) {
            File zone = new File(root, "zone");
            File main = new File(root, "main");
            if (zone.isDirectory() || main.isDirectory()) return true;
            error.append(zone).append(" (expected zone/<language>/common_mp.ff)\n")
                .append(main).append(" (expected multiplayer .iwd archives)\n");
        }
        error.append("Copy the zone and main folders from your MW2 multiplayer install into ")
            .append(candidates.iterator().next()).append(" and choose Start / retry.");
        StartupReport.write(dir, "failure.txt", error.toString());
        StartupReport.event(dir, "no game data directories; returning to setup before GPU initialization");
        return false;
    }

    static synchronized LinkedHashSet<File> prepare(Activity context, File dir) throws Exception {
        if (processAttempt != null && !dir.getName().equals(processAttempt)) {
            throw new IllegalStateException("The game process is still configured for an earlier attempt");
        }
        processAttempt = dir.getName();
        if (loadedRoots != null) return loadedRoots;
        StartupReport.boundAttempt = dir;
        StartupReport.write(dir, "engine.pid", String.valueOf(android.os.Process.myPid()));
        File privateFiles = context.getFilesDir();
        File files = context.getExternalFilesDir(null);
        if (files == null) files = privateFiles;
        File media = StartupReport.media(context);
        Os.setenv("IW4L_ANDROID_PRIVATE", privateFiles.getAbsolutePath(), true);
        Os.setenv("IW4L_ANDROID_FILES", files.getAbsolutePath(), true);
        Os.setenv("IW4L_ANDROID_MEDIA", media.getAbsolutePath(), true);
        Os.setenv("IW4L_STARTUP_DIR", dir.getAbsolutePath(), true);
        StartupReport.event(dir, "configuring runtime; media=" + media + ", files=" + files);
        RuntimeEnvironment.configure(media, files, privateFiles, dir);
        LinkedHashSet<File> roots = gameRoots(media, files, context.getExternalMediaDirs(),
            context.getExternalFilesDirs(null), Os.getenv("IW4L_GAMES"));
        org.json.JSONArray paths = new org.json.JSONArray();
        for (File root : roots) paths.put(root.getAbsolutePath());
        Os.setenv("IW4L_ANDROID_GAME_ROOTS", paths.toString(), true);
        StartupReport.event(dir, "MW2 search order: " + paths);
        StartupReport.event(dir, "System.loadLibrary(iw4l_android)");
        System.loadLibrary("iw4l_android");
        loadedRoots = roots;
        StartupReport.event(dir, "ARM64 library loaded");
        return loadedRoots;
    }
}
