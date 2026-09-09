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
    public static native long swarmJoin(String storePath, String nodeKeyPath);
    public static native String swarmNodeId(long handle);
    public static native String swarmTick(long handle, long maxAdopt);
    public static native void swarmLeave(long handle);
    public static native String vaultStatus(String vaultPath);
    public static native long vaultRewrap(String vaultPath);
    public static native long vaultGrant(String vaultPath, String identityPath, String readerPubHex, String purpose);
    public static native long vaultRevoke(String vaultPath, String identityPath, String readerPubHex, String purpose);
}
