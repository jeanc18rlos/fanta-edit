// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "FantaRevenueCatBridge",
    platforms: [.macOS(.v10_15)],
    products: [
        .library(name: "FantaRevenueCatBridge", type: .dynamic, targets: ["FantaRevenueCatBridge"])
    ],
    dependencies: [
        .package(url: "https://github.com/RevenueCat/purchases-ios-spm.git", exact: "5.82.0")
    ],
    targets: [
        .target(
            name: "FantaRevenueCatBridge",
            dependencies: [.product(name: "RevenueCat", package: "purchases-ios-spm")]
        )
    ]
)
