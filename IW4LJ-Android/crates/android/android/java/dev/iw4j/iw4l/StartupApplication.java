package dev.iw4j.iw4l;

import android.app.Application;
import android.util.Log;

import java.io.File;

public final class StartupApplication extends Application {
    @Override
    public void onCreate() {
        super.onCreate();
        final String process = getProcessName();
        Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, cause) -> {
            try {
                File dir = StartupReport.boundAttempt;
                if (dir == null) dir = getFilesDir();
                StartupReport.append(dir, "java-error.txt", process + " thread=" + thread.getName()
                    + "\n" + StartupReport.stack(cause));
            } finally {
                if (previous != null) previous.uncaughtException(thread, cause);
            }
        });
        Log.i("IW4LStartup", "Application created, process=" + process);
    }
}
