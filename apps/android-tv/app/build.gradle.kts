plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "org.opensanctuary.tv"
    compileSdk = 36

    defaultConfig {
        applicationId = "org.opensanctuary.tv"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        jvmToolchain(17)
    }

    buildFeatures {
        viewBinding = true
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    implementation("com.google.android.material:material:1.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    // Video playback for motion backgrounds
    implementation("androidx.media3:media3-exoplayer:1.3.1")
    implementation("androidx.media3:media3-ui:1.3.1")

    // QR encoding for the pairing-QR the TV displays in PairingManager's
    // manual-pairing fallback (docs/CLIENT_PAIRING.md's two-way bridge) --
    // core only, no camera/scanning needed on this side.
    implementation("com.google.zxing:core:3.5.3")

    // HTTP calls PairingManager makes itself (self-authorize, server-info,
    // pairing-status poll) -- lightweight, no reflection/codegen needed for
    // the handful of small JSON calls this makes.
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
