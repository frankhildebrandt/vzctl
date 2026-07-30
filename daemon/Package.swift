// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "vzctl-daemon",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "vz-supervisor", targets: ["VzSupervisor"]),
        .executable(name: "vz-helper", targets: ["VzHelper"]),
        .library(name: "VzDaemonKit", targets: ["VzDaemonKit"]),
    ],
    targets: [
        .target(name: "VzDaemonKit"),
        .executableTarget(
            name: "VzSupervisor",
            dependencies: ["VzDaemonKit"],
            linkerSettings: [
                .linkedFramework("vmnet"),
                .linkedFramework("Virtualization"),
            ]
        ),
        .executableTarget(
            name: "VzHelper",
            dependencies: ["VzDaemonKit"],
            linkerSettings: [
                .linkedFramework("Virtualization"),
                .linkedFramework("vmnet"),
            ]
        ),
    ]
)
