plugins {
    id("com.android.application")   // AGP 9: Kotlin support is built in
}

android {
    namespace = "de.wtzg.probewink"
    compileSdk = 36

    defaultConfig {
        applicationId = "de.wtzg.probewink"
        minSdk = 31            // Android 12: the BLUETOOTH_CONNECT/SCAN permission model
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Personal sideload build: signed with this machine's debug key, so
            // updates install over each other. Use a real key before sharing it.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        // English-only personal app: hard-coded UI strings are fine.
        disable += setOf("HardcodedText", "SetTextI18n", "ObsoleteSdkInt")
        warningsAsErrors = false
        abortOnError = true
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
}
