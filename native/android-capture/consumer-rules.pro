# kotlinx.serialization: keep generated serializers of the contract types.
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**
-keepclassmembers class dev.captureone.capture.** {
    *** Companion;
}
-keepclasseswithmembers class dev.captureone.capture.** {
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class dev.captureone.capture.**$$serializer { *; }
