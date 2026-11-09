package dev.iw4j.iw4l;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

final class EngineTeardown {
    private EngineTeardown() {}

    static void await(Runnable destroy, Runnable retire, long timeoutMillis) {
        CountDownLatch returned = new CountDownLatch(1);
        Thread deadline = new Thread(() -> {
            try {
                if (!returned.await(timeoutMillis, TimeUnit.MILLISECONDS)) retire.run();
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
            }
        }, "iw4l-engine-destroy");
        deadline.setDaemon(true);
        deadline.start();
        try {
            destroy.run();
        } finally {
            returned.countDown();
        }
    }
}
