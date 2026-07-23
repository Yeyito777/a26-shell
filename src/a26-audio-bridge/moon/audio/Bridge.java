package moon.audio;

import android.media.AudioFormat;
import android.media.AudioTrack;
import android.os.Looper;
import android.os.Process;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;

/**
 * A small Android-AudioTrack sink for Moon's Linux applications.
 *
 * <p>The process is deliberately created while Android's Java services are
 * still available. Its already-authorized AudioTrack remains connected to the
 * native AudioFlinger/HAL stack after Moon suspends system_server and takes DRM
 * master. Applications write 48 kHz stereo signed-16-bit PCM to a private FIFO.
 */
public final class Bridge {
    private static final int RATE = 48_000;
    private static final int CHANNELS = AudioFormat.CHANNEL_OUT_STEREO;
    private static final int ENCODING = AudioFormat.ENCODING_PCM_16BIT;
    private static final int FRAME_BYTES = 4;

    private Bridge() {}

    private static float readVolume(String path, float fallback) {
        byte[] bytes = new byte[16];
        try (FileInputStream input = new FileInputStream(path)) {
            int count = input.read(bytes);
            if (count <= 0) return fallback;
            String text = new String(bytes, 0, count, StandardCharsets.US_ASCII).trim();
            int value = Integer.parseInt(text);
            return Math.max(0, Math.min(100, value)) / 100.0f;
        } catch (Exception ignored) {
            return fallback;
        }
    }

    private static AudioTrack createAudioTrack(int bufferSize) throws Exception {
        // A bare app_process has no Context and AudioTrack cannot resolve an
        // AttributionSource without one. Construct a system resource context
        // before system_server is suspended, then pass it explicitly to the
        // public AudioTrack.Builder context API. Reflection keeps this tiny
        // bridge buildable against the pinned, redistributable Android stubs.
        if (Looper.myLooper() == null) Looper.prepare();
        Class<?> activityThreadClass = Class.forName("android.app.ActivityThread");
        Constructor<?> activityThreadConstructor = activityThreadClass.getDeclaredConstructor();
        activityThreadConstructor.setAccessible(true);
        Object activityThread = activityThreadConstructor.newInstance();
        Object context = activityThreadClass.getMethod("getSystemContext").invoke(activityThread);

        Class<?> formatBuilderClass = Class.forName("android.media.AudioFormat$Builder");
        Object formatBuilder = formatBuilderClass.getConstructor().newInstance();
        formatBuilderClass.getMethod("setEncoding", int.class).invoke(formatBuilder, ENCODING);
        formatBuilderClass.getMethod("setSampleRate", int.class).invoke(formatBuilder, RATE);
        formatBuilderClass.getMethod("setChannelMask", int.class).invoke(formatBuilder, CHANNELS);
        Object format = formatBuilderClass.getMethod("build").invoke(formatBuilder);

        Class<?> trackBuilderClass = Class.forName("android.media.AudioTrack$Builder");
        Object trackBuilder = trackBuilderClass.getConstructor().newInstance();
        trackBuilderClass.getMethod("setContext", Class.forName("android.content.Context"))
                .invoke(trackBuilder, context);
        trackBuilderClass.getMethod("setAudioFormat", Class.forName("android.media.AudioFormat"))
                .invoke(trackBuilder, format);
        trackBuilderClass.getMethod("setBufferSizeInBytes", int.class)
                .invoke(trackBuilder, bufferSize);
        trackBuilderClass.getMethod("setTransferMode", int.class)
                .invoke(trackBuilder, AudioTrack.MODE_STREAM);
        return (AudioTrack) trackBuilderClass.getMethod("build").invoke(trackBuilder);
    }

    private static void writePid(File pidFile) throws IOException {
        try (FileOutputStream output = new FileOutputStream(pidFile, false)) {
            output.write((Integer.toString(Process.myPid()) + "\n")
                    .getBytes(StandardCharsets.US_ASCII));
            output.getFD().sync();
        }
    }

    private static void removeOwnPid(File pidFile) {
        byte[] bytes = new byte[32];
        try (FileInputStream input = new FileInputStream(pidFile)) {
            int count = input.read(bytes);
            if (count <= 0) return;
            String value = new String(bytes, 0, count, StandardCharsets.US_ASCII).trim();
            if (value.equals(Integer.toString(Process.myPid()))) pidFile.delete();
        } catch (Exception ignored) {
            // Shutdown cleanup is best-effort; the launcher validates stale PIDs.
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 3) {
            System.err.println("usage: Bridge PCM_FIFO VOLUME_FILE PID_FILE");
            System.exit(2);
        }
        String pcmPath = args[0];
        String volumePath = args[1];
        File pidFile = new File(args[2]);

        int minimum = AudioTrack.getMinBufferSize(RATE, CHANNELS, ENCODING);
        if (minimum <= 0) throw new IOException("AudioTrack buffer query failed: " + minimum);
        int bufferSize = Math.max(minimum, RATE * FRAME_BYTES / 5);
        AudioTrack track = createAudioTrack(bufferSize);
        if (track.getState() != AudioTrack.STATE_INITIALIZED) {
            throw new IOException("AudioTrack initialization failed");
        }

        float volume = readVolume(volumePath, 0.5f);
        track.setStereoVolume(volume, volume);
        writePid(pidFile);
        Runtime.getRuntime().addShutdownHook(new Thread(() -> removeOwnPid(pidFile)));
        System.err.println("moon audio bridge ready rate=48000 channels=2 format=s16le");

        byte[] buffer = new byte[3_840 + FRAME_BYTES];
        try {
            while (true) {
                int carried = 0;
                boolean playing = false;
                try (FileInputStream input = new FileInputStream(new File(pcmPath))) {
                    int count;
                    while ((count = input.read(buffer, carried, buffer.length - carried)) >= 0) {
                        if (count == 0) continue;
                        count += carried;
                        int aligned = count - (count % FRAME_BYTES);
                        carried = count - aligned;

                        float nextVolume = readVolume(volumePath, volume);
                        if (nextVolume != volume) {
                            volume = nextVolume;
                            track.setStereoVolume(volume, volume);
                        }
                        if (!playing) {
                            track.play();
                            playing = true;
                        }
                        int offset = 0;
                        while (offset < aligned) {
                            int wrote = track.write(buffer, offset, aligned - offset);
                            if (wrote < 0) {
                                throw new IOException("AudioTrack write failed: " + wrote);
                            }
                            offset += wrote;
                        }
                        if (carried > 0) {
                            System.arraycopy(buffer, aligned, buffer, 0, carried);
                        }
                    }
                }
                if (playing) {
                    track.pause();
                    track.flush();
                }
            }
        } finally {
            try {
                track.stop();
            } catch (IllegalStateException ignored) {
                // A track with no submitted audio is already stopped.
            }
            track.release();
            removeOwnPid(pidFile);
        }
    }
}
