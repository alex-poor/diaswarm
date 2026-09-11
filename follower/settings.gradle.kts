// A STANDALONE GRADLE BUILD, and that is the point.
//
// The follower used to be a flavour of AndroidAPS: a whole loop app with the
// dosing stripped out. That worked, and it was the wrong thing to hand to
// somebody. AAPS is deliberately not on app stores — running a loop means
// assembling a medical device yourself — and shipping something that is
// visibly an AAPS build to an audience AAPS does not serve muddies exactly the
// line this project should be keeping clear.
//
// So this contains no AndroidAPS at all. Not a stripped copy, not a dependency:
// none. The safety argument stops being "a build flavour that cannot dose" and
// becomes "there is no dosing code in the binary", which is the version that
// survives somebody reading the manifest.
//
// It also makes the thing publishable: one repository, no `srclibs`, no
// committed .aar inherited from upstream, no versionCode shared with the loop
// app, and a build measured in seconds rather than a full AAPS compile.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // No jitpack. Nothing reproducible comes out of it, and F-Droid will
        // not build against it.
    }
}
rootProject.name = "ayni"
