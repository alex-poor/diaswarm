# The JNI boundary. R8 cannot see that the native library calls back into these
# names, so without this the methods are renamed and every call fails at run
# time with UnsatisfiedLinkError — on a phone, not in a build.
-keep class nz.diaswarm.jni.SwarmNative { *; }

# zxing's scanner reaches for classes reflectively.
-keep class com.journeyapps.barcodescanner.** { *; }
-keep class com.google.zxing.** { *; }

# WorkManager instantiates workers by name from its own database, including
# after a reboot, so a renamed worker is a sync that silently stops.
-keep class * extends androidx.work.Worker { *; }
-keep class * extends androidx.work.ListenableWorker { *; }
