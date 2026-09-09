import app.aaps.plugins.sync.swarm.SwarmNative;

public class Check {
    static int failures = 0;

    static void check(String name, boolean ok, String detail) {
        System.out.printf("  %-4s %s%s%n", ok ? "ok" : "FAIL", name, ok ? "" : "   " + detail);
        if (!ok) failures++;
    }

    public static void main(String[] args) {
        System.loadLibrary("diaswarm_android");

        check("spec version crosses the boundary", SwarmNative.specVersion() == 3,
              "got " + SwarmNative.specVersion());
        check("epoch arithmetic is the core's", SwarmNative.epochOf(86_400_000L, 0L) == 1,
              "got " + SwarmNative.epochOf(86_400_000L, 0L));
        check("negative timestamps floor", SwarmNative.epochOf(-1L, 0L) == -1,
              "got " + SwarmNative.epochOf(-1L, 0L));
        check("the offset moves the phase, not the width",
              SwarmNative.epochOf(43200000L - 1, 43200000L) == 0
              && SwarmNative.epochOf(43200000L, 43200000L) == 1,
              "a +12h offset must put the boundary at local midnight");

        String header = SwarmNative.header(43200000L);
        check("header is the canonical v2 header",
              header.equals("{\"epoch\":\"offset-day\",\"k\":\"meta\",\"offset\":43200000,\"spec\":3,\"t\":0,\"unit\":\"mgdl\"}"),
              header);

        // Keys unsorted, a null field, and an unrounded double: all three are
        // things Kotlin will actually hand over.
        String raw = "{\"t\":1782938503230,\"k\":\"cgm\",\"src\":\"Dexcom G6\","
                   + "\"mgdl\":163.04999,\"trend\":null}";
        String line = SwarmNative.canonicalLine(raw);
        check("keys sorted, null dropped, value rounded",
              line.equals("{\"k\":\"cgm\",\"mgdl\":163.0,\"src\":\"Dexcom G6\",\"t\":1782938503230}"),
              line);

        check("a malformed record returns empty, not an exception",
              SwarmNative.canonicalLine("not json").isEmpty(), "threw or returned content");

        long e = SwarmNative.emitterNew();
        check("emitter allocates", e != 0, "got 0");
        String first = SwarmNative.emitterAccept(e, raw);
        String again = SwarmNative.emitterAccept(e, raw);
        check("first sight of a record is emitted", !first.isEmpty(), "empty");
        check("the same record again is not", again.isEmpty(), again);

        // Same instant and kind, different value: a genuine edit, counted.
        String edited = "{\"t\":1782938503230,\"k\":\"cgm\",\"mgdl\":170.0}";
        SwarmNative.emitterAccept(e, edited);
        check("a real edit is counted, not silently dropped",
              SwarmNative.emitterAmendments(e) == 1,
              "got " + SwarmNative.emitterAmendments(e));

        // --- the vault ------------------------------------------------------
        String base = System.getProperty("java.io.tmpdir") + "/diaswarm-jni-" + System.nanoTime();
        String vault = base + "/vault";
        String ident = base + "/subject.id";

        String subject = SwarmNative.vaultSubject(ident);
        check("an identity is created on first use", subject.length() == 64, subject);
        check("and is stable across calls", subject.equals(SwarmNative.vaultSubject(ident)), "changed");

        String day = "{\"k\":\"cgm\",\"mgdl\":163.0,\"t\":1782938503230}\n"
                   + "{\"k\":\"cgm\",\"mgdl\":164.0,\"t\":1782938803230}\n";
        long sealed = SwarmNative.vaultSeal(vault, ident, 20630L, 43200000L, day);
        check("a day seals through JNI", sealed == 2, "returned " + sealed);

        String status = SwarmNative.vaultStatus(vault);
        check("the vault reports itself", status.startsWith("1 epochs"), status);

        check("a bad vault path fails with a code, not an exception",
              SwarmNative.vaultSeal("/proc/nonexistent/vault", ident, 1L, 0L, day) < 0, "did not fail");
        check("an unopenable vault reports rather than throws",
              SwarmNative.vaultStatus("/proc/nonexistent").equals("no vault"),
              SwarmNative.vaultStatus("/proc/nonexistent"));

        // --- granting, from the phone side --------------------------------
        String reader = "11".repeat(32);   // a plausible 32-byte public key
        long wraps = SwarmNative.vaultGrant(vault, ident, reader, "follow");
        check("a grant publishes wraps for the sealed segments", wraps == 1,
              "returned " + wraps);
        check("a bad reader key is refused with a code",
              SwarmNative.vaultGrant(vault, ident, "nothex", "follow") == -1, "not refused");

        long from = SwarmNative.vaultRevoke(vault, ident, reader, "follow");
        check("a withdrawal returns the segment it starts at", from >= 0, "returned " + from);

        // --- serving ------------------------------------------------------
        long net = SwarmNative.netStart(vault, base + "/node.key");
        check("an endpoint starts", net != 0, "handle was 0");
        String eid = SwarmNative.netEndpointId(net);
        check("and reports a dialable id", eid.length() == 64, eid);
        check("the id is stable across calls", eid.equals(SwarmNative.netEndpointId(net)), "changed");
        SwarmNative.netStop(net);
        SwarmNative.netStop(0);   // must be a no-op, not a crash
        check("stopping twice does not crash", true, "");

        SwarmNative.emitterFree(e);
        check("freeing twice does not crash", freeTwice(), "crashed");

        System.out.println(failures == 0 ? "\n  the JNI contract holds\n"
                                         : "\n  " + failures + " FAILED\n");
        System.exit(failures == 0 ? 0 : 1);
    }

    static boolean freeTwice() {
        long h = SwarmNative.emitterNew();
        SwarmNative.emitterFree(h);
        SwarmNative.emitterFree(0); // the null handle must be a no-op
        return true;
    }
}
