// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "vzctl-daemon",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "vz-supervisor", targets: ["VzSupervisor"]),
        .executable(name: "vz-helper", targets: ["VzHelper"]),
        .executable(name: "vz-dns-bind", targets: ["VzDnsBind"]),
        .executable(name: "vz-net", targets: ["VzNet"]),
        .library(name: "VzDaemonKit", targets: ["VzDaemonKit"]),
    ],
    targets: [
        .target(
            name: "VzDaemonKit",
            linkerSettings: [
                .linkedFramework("vmnet"),
            ]
        ),
        .executableTarget(
            name: "VzSupervisor",
            dependencies: ["VzDaemonKit"],
            linkerSettings: [
                .linkedLibrary("sqlite3"),
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
        .executableTarget(
            name: "VzDnsBind",
            dependencies: ["VzDaemonKit"]
        ),
        .executableTarget(
            name: "VzNet",
            dependencies: ["VzDaemonKit"],
            linkerSettings: [
                .linkedFramework("vmnet"),
            ]
        ),
        .testTarget(
            name: "VzDaemonKitTests",
            dependencies: ["VzDaemonKit"]
        ),
        .testTarget(
            name: "VzHelperTests",
            dependencies: ["VzHelper"]
        ),
        .testTarget(
            name: "VzSupervisorTests",
            dependencies: ["VzSupervisor", "VzDaemonKit"]
        ),
        .testTarget(
            name: "VzNetTests",
            dependencies: ["VzNet", "VzDaemonKit"]
        ),
    ]
)
