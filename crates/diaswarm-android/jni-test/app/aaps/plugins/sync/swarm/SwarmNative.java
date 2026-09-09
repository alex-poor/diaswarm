package app.aaps.plugins.sync.swarm;

/** Mirrors the Kotlin object's external declarations, to exercise the real .so. */
public class SwarmNative {
    public static native int specVersion();
    public static native long epochOf(long t, long offsetMs);
    public static native String header(long offsetMs);
    public static native String canonicalLine(String json);
    public static native long emitterNew();
    public static native void emitterFree(long handle);
    public static native String emitterAccept(long handle, String json);
    public static native long emitterAmendments(long handle);
    public static native long vaultSeal(String vaultPath, String identityPath, long epoch, long offsetMs, String ndjson);
    public static native String vaultSubject(String identityPath);
    public static native String vaultStatus(String vaultPath);
}
