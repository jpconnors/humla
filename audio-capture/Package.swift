// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "audio-capture",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "audio-capture",
            path: "Sources/audio-capture",
            linkerSettings: [
                // Embed Info.plist into the binary's __TEXT,__info_plist
                // section so LSUIElement is in effect before AppKit
                // initializes. Without this, NSApplication.shared briefly
                // registers a Dock icon before setActivationPolicy(.prohibited)
                // takes hold on some macOS versions — the tester saw multiple
                // Humla icons flash in the Dock each time a permission status
                // check spawned the sidecar.
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Info.plist",
                ])
            ]
        )
    ]
)
