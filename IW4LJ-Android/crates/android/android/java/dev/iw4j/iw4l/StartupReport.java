package dev.iw4j.iw4l;

import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.os.Build;
import android.os.SystemClock;
import android.util.Log;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.io.RandomAccessFile;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Date;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.TimeUnit;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

final class StartupReport {
    static final String EXTRA = "startup_attempt";
    static volatile File boundAttempt;
    private static final String PREFIX = "attempt-";
    private static final int KIB = 1024;

    static File media(Context context) {
        File[] volumes = context.getExternalMediaDirs();
        if (volumes.length > 0 && volumes[0] != null) return volumes[0];
        File external = context.getExternalFilesDir(null);
        return external != null ? external : context.getFilesDir();
    }

    private static File base(Context context) {
        return new File(context.getFilesDir(), "startup");
    }

    private static boolean valid(String id) {
        return id != null && id.matches("attempt-[0-9]+-[a-f0-9-]{36}");
    }

    static File from(Intent intent, Context context) {
        String id = intent.getStringExtra(EXTRA);
        if (!valid(id)) throw new IllegalStateException("Missing private engine launch ID");
        return new File(base(context), id);
    }

    static File latest(Context context) {
        String id = read(base(context), "current", 200);
        return valid(id) ? new File(base(context), id) : null;
    }

    private static File[] attempts(Context context) {
        File[] dirs = base(context).listFiles(file -> file.isDirectory() && valid(file.getName()));
        if (dirs == null) return new File[0];
        Arrays.sort(dirs, Comparator.comparing(File::getName).reversed());
        return dirs;
    }

    static File begin(Context context) throws IOException {
        long now = System.currentTimeMillis();
        File dir = new File(base(context), PREFIX + now + "-" + UUID.randomUUID());
        if (!dir.mkdirs()) throw new IOException("Cannot create " + dir);
        write(dir, "started.ms", Long.toString(now));
        write(dir, "device.txt", identity(context));
        write(base(context), "current", dir.getName());
        boundAttempt = dir;
        event(dir, "launcher requested engine start");
        // Only files owned by this bounded diagnostic store can be rotated.
        File[] all = attempts(context);
        for (int index = 6; index < all.length; index++) {
            if (dir.equals(all[index])) continue;
            try {
                File[] entries = all[index].listFiles(f -> !f.isDirectory());
                if (entries != null) for (File file : entries) Files.deleteIfExists(file.toPath());
                Files.deleteIfExists(all[index].toPath());
            } catch (IOException error) {
                Log.w("IW4LStartup", "Could not rotate older diagnostics", error);
            }
        }
        return dir;
    }

    private static String identity(Context context) {
        String version;
        try {
            PackageInfo info = context.getPackageManager().getPackageInfo(context.getPackageName(), 0);
            version = info.versionName + " (" + info.getLongVersionCode() + ")";
        } catch (Exception ex) {
            version = ex.toString();
        }
        String revision = "";
        try (java.io.BufferedReader input = new java.io.BufferedReader(new java.io.InputStreamReader(
                context.getAssets().open("iw4l-source.txt"), StandardCharsets.UTF_8))) {
            String sha = input.readLine();
            if (sha != null && !sha.isEmpty()) revision = "\nSource " + sha;
        } catch (IOException missingMetadata) {
            // Older packages have no embedded revision.
        }
        return context.getPackageName() + " " + version + revision + "\n" + Build.MANUFACTURER + " " + Build.MODEL
            + " Android " + Build.VERSION.RELEASE + " API " + Build.VERSION.SDK_INT
            + "\nABI " + String.join(", ", Build.SUPPORTED_ABIS) + "; " + Build.SOC_MODEL;
    }

    static String stack(Throwable error) {
        StringWriter text = new StringWriter();
        error.printStackTrace(new PrintWriter(text));
        return text.toString();
    }

    static void write(File dir, String name, String value) throws IOException {
        File temp = File.createTempFile(".writing-", null, dir);
        try {
            Files.write(temp.toPath(), value.getBytes(StandardCharsets.UTF_8));
            Files.move(temp.toPath(), new File(dir, name).toPath(),
                StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } finally {
            temp.delete();
        }
    }

    static void append(File dir, String name, String line) {
        if (dir == null) return;
        try (FileOutputStream out = new FileOutputStream(new File(dir, name), true)) {
            out.write((line + "\n").getBytes(StandardCharsets.UTF_8));
        } catch (Exception ex) {
            Log.w("IW4LStartup", "Could not save " + name, ex);
        }
    }

    static void event(File dir, String text) {
        Log.i("IW4LStartup", text);
        append(dir, "java.log", System.currentTimeMillis() + " elapsed=" + SystemClock.elapsedRealtime()
            + " pid=" + android.os.Process.myPid() + " " + text);
    }

    static String read(File dir, String name, int max) {
        if (dir == null) return "";
        File file = new File(dir, name);
        if (!file.isFile()) return "";
        try (RandomAccessFile input = new RandomAccessFile(file, "r")) {
            int count = (int) Math.min(input.length(), max);
            boolean shortened = input.length() > count;
            input.seek(input.length() - count);
            byte[] bytes = new byte[count];
            input.readFully(bytes);
            return (shortened ? "[earlier bytes omitted]\n" : "")
                + new String(bytes, StandardCharsets.UTF_8).trim();
        } catch (Exception ex) {
            Log.w("IW4LStartup", "Could not read " + file, ex);
            return "";
        }
    }

    static boolean present(File dir, String name) {
        return dir != null && new File(dir, name).length() > 0;
    }

    static String summary(Context context, File dir) {
        StringBuilder out = new StringBuilder();
        out.append("Default MW2 multiplayer folder for zone/ and main/:\n")
            .append(new File(media(context), "mw2"))
            .append("\nStart game also checks accessible installations on mounted SD cards.\n\n");
        if (dir == null) {
            out.append(identity(context)).append("\nPress Start game to check the files and open the MW2 menu.\n");
        } else {
            section(out, dir, "failure.txt", "Engine startup or render error", 12 * KIB);
            section(out, dir, "java-error.txt", "Java / library error", 8 * KIB);
            section(out, dir, "crash.txt", "Rust panic", 8 * KIB);
            section(out, dir, "stage.txt", "Last native stage", 2 * KIB);
            section(out, dir, "ready.txt", "Menu presentation", 2 * KIB);
            if (!present(dir, "ready.txt")) out.append("Menu presentation was not recorded.\n\n");
            section(out, dir, "device.txt", "Build and device", KIB);
            section(out, dir, "exit-info.txt", "Android process exit history", 4 * KIB);
            section(out, dir, "native.log", "Native startup events", 12 * KIB);
            section(out, dir, "java.log", "Android startup events", 8 * KIB);
            out.append("Export ZIP includes saved traces, engine logs and earlier attempts.\n");
        }
        section(out, context.getFilesDir(), "java-error.txt", "Java error outside a game attempt", 4 * KIB);
        return out.toString();
    }

    private static void section(StringBuilder out, File dir, String name, String title, int max) {
        String text = read(dir, name, max);
        if (!text.isEmpty()) out.append(title).append("\n").append(text).append("\n\n");
    }

    private static String reason(int reason) {
        switch (reason) {
            case ApplicationExitInfo.REASON_ANR: return "ANR";
            case ApplicationExitInfo.REASON_CRASH: return "Java crash";
            case ApplicationExitInfo.REASON_CRASH_NATIVE: return "native crash";
            case ApplicationExitInfo.REASON_SIGNALED: return "signal";
            case ApplicationExitInfo.REASON_EXIT_SELF: return "self-exit";
            case ApplicationExitInfo.REASON_LOW_MEMORY: return "low memory";
            case ApplicationExitInfo.REASON_USER_REQUESTED: return "user requested";
            case ApplicationExitInfo.REASON_INITIALIZATION_FAILURE: return "initialization failure";
            case ApplicationExitInfo.REASON_USER_STOPPED: return "user stopped";
            case ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE: return "excessive resource usage";
            default: return "Android reason " + reason;
        }
    }

    static void collect(Context context, File dir) {
        if (dir == null) return;
        try {
            long since = Long.parseLong(read(dir, "started.ms", 64));
            String pidText = read(dir, "engine.pid", 16);
            int pid = pidText.isEmpty() ? 0 : Integer.parseInt(pidText);
            String engine = context.getPackageName() + ":engine";
            File next = null;
            for (File attempt : attempts(context)) {
                if (attempt.equals(dir)) break;
                next = attempt;
            }
            long end = next == null ? Long.MAX_VALUE : Long.parseLong(read(next, "started.ms", 64));
            List<ApplicationExitInfo> exits = context.getSystemService(ActivityManager.class)
                .getHistoricalProcessExitReasons(context.getPackageName(), pid, 32);
            for (ApplicationExitInfo exit : exits) {
                if (exit.getTimestamp() < since || exit.getTimestamp() >= end
                    || !engine.equals(exit.getProcessName())) continue;
                String id = exit.getTimestamp() + "-" + exit.getPid();
                String record = "android-exit-" + id + ".txt";
                StringBuilder status = new StringBuilder();
                status.append(new Date(exit.getTimestamp())).append(" pid=").append(exit.getPid())
                    .append(' ').append(reason(exit.getReason())).append(" status=").append(exit.getStatus())
                    .append(" pss=").append(exit.getPss()).append(" KiB rss=").append(exit.getRss())
                    .append(" KiB\n").append(exit.getDescription()).append('\n');
                File trace = new File(dir, "android-trace-" + id + ".bin");
                if (!trace.exists()) {
                    try (InputStream input = exit.getTraceInputStream()) {
                        if (input != null) {
                            File temp = File.createTempFile(".trace-", null, dir);
                            try {
                                try (FileOutputStream out = new FileOutputStream(temp)) {
                                    if (copy(input, out, 4 * 1024 * KIB)) {
                                        status.append("Trace capped at 4 MiB.\n");
                                    }
                                }
                                Files.move(temp.toPath(), trace.toPath(), StandardCopyOption.ATOMIC_MOVE,
                                    StandardCopyOption.REPLACE_EXISTING);
                            } finally {
                                temp.delete();
                            }
                        }
                    } catch (Exception ex) {
                        status.append("Trace unavailable: ").append(ex).append('\n');
                    }
                }
                if (!present(dir, record)) write(dir, record, status.toString());
            }
            File[] records = dir.listFiles(f -> f.getName().startsWith("android-exit-") && f.isFile());
            if (records != null && records.length > 0) {
                Arrays.sort(records, Comparator.comparing(File::getName).reversed());
                StringBuilder history = new StringBuilder();
                for (File record : records) history.append(read(dir, record.getName(), 8 * KIB)).append('\n');
                write(dir, "exit-info.txt", history.toString());
            } else if (!present(dir, "exit-info.txt")) {
                write(dir, "exit-info.txt", "No matching Android exit record available.");
            }
            if (pid > 0 && !present(dir, "logcat.txt")) captureLogcat(dir, pid);
        } catch (Exception ex) {
            event(dir, "Exit evidence unavailable: " + ex);
        }
    }

    private static void captureLogcat(File dir, int pid) throws Exception {
        File tmp = File.createTempFile(".logcat-", null, dir);
        java.lang.Process process = null;
        try {
            process = new ProcessBuilder("logcat", "-d", "--pid=" + pid, "-t", "2500", "-v", "threadtime")
                .redirectErrorStream(true).redirectOutput(tmp).start();
            if (!process.waitFor(4, TimeUnit.SECONDS)) process.destroyForcibly();
            String lines = read(dir, tmp.getName(), 1024 * KIB);
            if (!lines.isEmpty()) write(dir, "logcat.txt", lines);
        } finally {
            if (process != null && process.isAlive()) process.destroyForcibly();
            tmp.delete();
        }
    }

    static void export(Context context, OutputStream target) throws IOException {
        for (File attempt : attempts(context)) collect(context, attempt);
        try (ZipOutputStream zip = new ZipOutputStream(target)) {
            File latest = latest(context);
            zip.putNextEntry(new ZipEntry("summary.txt"));
            zip.write(summary(context, latest).getBytes(StandardCharsets.UTF_8));
            zip.closeEntry();
            for (File dir : attempts(context)) {
                File[] entries = dir.listFiles(f -> f.isFile() && !f.getName().startsWith(".")
                    && !Files.isSymbolicLink(f.toPath()));
                if (entries == null) continue;
                for (File entry : entries) {
                    zip.putNextEntry(new ZipEntry(dir.getName() + "/" + entry.getName()));
                    try (InputStream in = new FileInputStream(entry)) {
                        copy(in, zip, Long.MAX_VALUE);
                    }
                    zip.closeEntry();
                }
            }
        }
    }

    private static boolean copy(InputStream in, OutputStream out, long limit) throws IOException {
        byte[] buffer = new byte[8192];
        int count;
        while (limit > 0 && (count = in.read(buffer, 0, (int) Math.min(limit, buffer.length))) != -1) {
            out.write(buffer, 0, count);
            limit -= count;
        }
        return limit == 0 && in.read() != -1;
    }
}
