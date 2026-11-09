package dev.iw4j.iw4l;

import android.system.Os;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.Locale;

final class RuntimeEnvironment {
    private RuntimeEnvironment() {}

    static void configure(File media, File files, File privateFiles, File attempt) throws Exception {
        File config = new File(privateFiles, "config");
        Files.createDirectories(config.toPath());
        File[] dirs = {media, files};
        if ("0".equals(text(new File(files, "sync_mode.txt")))) {
            Os.setenv("IW4L_PRESENT_MODE", "AutoNoVsync", false);
        }
        Os.setenv("IW4L_PIPELINE_CACHE", new File(files, "pipeline_cache.bin").getPath(), false);
        for (File base : dirs) {
            for (String line : text(new File(base, "env.txt")).split("\n")) {
                int equals = line.indexOf('=');
                if (equals < 1 || line.contains("give weapon/iw4:weapon/uzi_mp akimbo")) continue;
                String name = line.substring(0, equals).trim();
                if (name.matches("IW4L_[A-Za-z0-9_]+") && !name.startsWith("IW4L_ANDROID_")
                        && !name.equals("IW4L_STARTUP_DIR")) {
                    Os.setenv(name, line.substring(equals + 1).trim(), true);
                }
            }
        }
        Os.setenv("HOME", files.getPath(), true);
        Os.setenv("XDG_CONFIG_HOME", config.getPath(), true);
        Os.setenv("IW4L_ARTIFACTS_DIR", new File(files, "iw4l-artifacts").getPath(), true);
        Os.setenv("IW4L_LOG", new File(attempt, "native.log").getPath(), true);
        String texture = text(new File(files, "texture_skip.txt"));
        if (!texture.isEmpty()) Os.setenv("IW4L_TEXTURE_SKIP", texture, false);
        for (File base : dirs) {
            String commands = text(new File(base, "cmds.txt"));
            if (!commands.isEmpty() && !commands.contains("map mp_terminal; wait world; spawn 0; force_match_start; bot add 1")) {
                Os.setenv("IW4L_CMDS", commands, false);
                break;
            }
        }
        Os.setenv("IW4L_CMDS", "wait progression; unlock all", false);
        for (File base : dirs) {
            File[] descriptors = base.listFiles((dir, name) ->
                name.toLowerCase(Locale.ROOT).endsWith(".iw4l-server"));
            if (descriptors != null && descriptors.length != 0) {
                Os.setenv("IW4L_COMMUNITY", descriptors[0].getPath(), false);
                break;
            }
        }
        Os.setenv("IW4L_COMMUNITY_ROOT", config.getPath(), true);
        Os.setenv("IW4L_COMMUNITY", new File(config, "lan.iw4l-server").getPath(), false);
    }

    static byte[] nativeSettings() throws IOException {
        ByteArrayOutputStream result = new ByteArrayOutputStream();
        for (String entry : Os.environ()) {
            int equals = entry.indexOf('=');
            if (equals <= 0) continue;
            String key = entry.substring(0, equals);
            if (key.equals("HOME") || key.equals("XDG_CONFIG_HOME") || key.matches("IW4L_[a-zA-Z0-9_]+")) {
                result.write(entry.getBytes(StandardCharsets.UTF_8));
                result.write(0);
            }
        }
        return result.toByteArray();
    }

    private static String text(File file) throws IOException {
        if (!file.exists()) return "";
        try (FileInputStream input = new FileInputStream(file);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] bytes = new byte[4096];
            int count;
            while ((count = input.read(bytes)) != -1) {
                if (output.size() + count > 1024 * 1024) {
                    throw new IOException("Startup settings exceed 1 MiB: " + file);
                }
                output.write(bytes, 0, count);
            }
            return new String(output.toByteArray(), StandardCharsets.UTF_8).trim();
        }
    }
}
