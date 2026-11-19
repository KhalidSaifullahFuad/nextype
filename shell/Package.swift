// swift-tools-version:6.0
import PackageDescription
import Foundation

// The Rust core is linked as a static library built by scripts/bundle.sh.
let rustLib = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("target/release").path

let package = Package(
    name: "NexType",
    platforms: [.macOS(.v14)],
    targets: [
        // C module holding the uniffi-generated header; a C target needs one source file.
        .target(name: "nextype_ffiFFI", path: "Sources/nextype_ffiFFI", publicHeadersPath: "include"),
        .executableTarget(
            name: "NexType",
            dependencies: ["nextype_ffiFFI"],
            path: "Sources/NexType",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-L", rustLib]),
                .linkedLibrary("nextype_ffi"),
                .linkedLibrary("c++"),
                .linkedFramework("Accelerate"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("AppKit"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("Vision"),
                .linkedFramework("Security"),
            ]
        ),
    ]
)
