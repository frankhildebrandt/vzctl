// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "G0Spike",
    platforms: [
        .macOS(.v26),
    ],
    targets: [
        .executableTarget(
            name: "G0Spike",
            path: "Sources/G0Spike",
            swiftSettings: [
                .unsafeFlags(["-parse-as-library"]),
            ],
            linkerSettings: [
                .linkedFramework("vmnet"),
                .linkedFramework("Virtualization"),
                .linkedFramework("Network"),
            ]
        ),
    ]
)
