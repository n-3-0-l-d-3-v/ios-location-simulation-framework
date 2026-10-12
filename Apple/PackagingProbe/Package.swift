// swift-tools-version:5.9
//
// EXPERIMENTAL. A fixture that proves one thing: that the XCFramework made
// by Scripts/apple/build-xcframework.sh can be consumed by a Swift package
// and its tests. It is not the Swift adapter of T11B and exposes no API.
//
// The binary target and the test fixture are generated, not checked in.
// Run the script first; without it this package does not resolve.
import PackageDescription

let package = Package(
    name: "LocsimPackagingProbe",
    // A value the probe needs in order to build. It is not the minimum iOS
    // version of the project: that decision has not been made.
    platforms: [.iOS(.v16)],
    products: [
        .library(name: "LocsimPackagingProbe", targets: ["LocsimPackagingProbe"])
    ],
    targets: [
        .binaryTarget(name: "LocsimFFI", path: "Artifacts/LocsimFFI.xcframework"),
        .target(name: "LocsimPackagingProbe", dependencies: ["LocsimFFI"]),
        .testTarget(
            name: "LocsimPackagingProbeTests",
            dependencies: ["LocsimPackagingProbe", "LocsimFFI"],
            resources: [.copy("Fixtures")]
        ),
    ]
)
