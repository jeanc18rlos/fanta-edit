import Foundation
import RevenueCat

public typealias Completion = @convention(c) (
    UnsafeMutableRawPointer?,
    Int32,
    UnsafePointer<CChar>?
) -> Void

private enum State {
    static var publicAPIKey: String?
    static var appUserID: String?
}

private struct SendableContext: @unchecked Sendable {
    let pointer: UnsafeMutableRawPointer?
}

private func complete(
    _ callback: Completion,
    context: UnsafeMutableRawPointer?,
    status: Int32,
    text: String
) {
    text.withCString { pointer in
        callback(context, status, pointer)
    }
}

private func completeJSON(
    _ callback: Completion,
    context: UnsafeMutableRawPointer?,
    object: Any
) {
    do {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.fragmentsAllowed])
        guard let text = String(data: data, encoding: .utf8) else {
            complete(callback, context: context, status: 2, text: "Unable to encode RevenueCat response")
            return
        }
        complete(callback, context: context, status: 0, text: text)
    } catch {
        complete(callback, context: context, status: 2, text: error.localizedDescription)
    }
}

private func productJSON(_ product: StoreProduct) -> [String: Any] {
    [
        "identifier": product.productIdentifier,
        "title": product.localizedTitle,
        "description": product.localizedDescription,
        "localized_price": product.localizedPriceString,
        "currency_code": product.currencyCode ?? ""
    ]
}

private func customerJSON(_ info: CustomerInfo) -> [String: Any] {
    [
        "app_user_id": Purchases.shared.appUserID,
        "active_entitlements": Array(info.entitlements.active.keys).sorted(),
        "active_subscriptions": Array(info.activeSubscriptions).sorted(),
        "purchased_product_identifiers": Array(info.allPurchasedProductIdentifiers).sorted()
    ]
}

private func completeCustomer(
    _ callback: Completion,
    context: UnsafeMutableRawPointer?,
    info: CustomerInfo?,
    error: Error?
) {
    if let error {
        complete(callback, context: context, status: 2, text: error.localizedDescription)
    } else if let info {
        completeJSON(callback, context: context, object: customerJSON(info))
    } else {
        complete(callback, context: context, status: 2, text: "RevenueCat returned no customer information")
    }
}

@_cdecl("fanta_rc_configure")
public func fanta_rc_configure(
    _ publicAPIKey: UnsafePointer<CChar>?,
    _ appUserID: UnsafePointer<CChar>?
) -> Int32 {
    guard Thread.isMainThread else { return 1 }
    guard
        let publicAPIKey,
        let appUserID
    else { return 3 }

    let key = String(cString: publicAPIKey)
    let userID = String(cString: appUserID)
    guard !key.isEmpty, !userID.isEmpty else { return 3 }
    if let configuredKey = State.publicAPIKey {
        if configuredKey != key { return 2 }
        return State.appUserID == userID ? 0 : 4
    }

    Purchases.configure(withAPIKey: key, appUserID: userID)
    State.publicAPIKey = key
    State.appUserID = userID
    return 0
}

@_cdecl("fanta_rc_request")
public func fanta_rc_request(
    _ operation: UnsafePointer<CChar>?,
    _ argument: UnsafePointer<CChar>?,
    _ context: UnsafeMutableRawPointer?,
    _ callback: Completion
) {
    guard let operation else {
        complete(callback, context: context, status: 2, text: "Missing RevenueCat operation")
        return
    }
    let name = String(cString: operation)
    let value = argument.map { String(cString: $0) }

    DispatchQueue.main.async {
        guard State.publicAPIKey != nil else {
            complete(callback, context: context, status: 2, text: "RevenueCat is not configured")
            return
        }

        switch name {
        case "log_in":
            guard let userID = value, !userID.isEmpty else {
                complete(callback, context: context, status: 2, text: "Missing app user ID")
                return
            }
            Purchases.shared.logIn(userID) { info, _, error in
                if error == nil, info != nil {
                    State.appUserID = userID
                }
                completeCustomer(callback, context: context, info: info, error: error)
            }
        case "log_out":
            Purchases.shared.logOut { info, error in
                if error == nil, info != nil {
                    State.appUserID = nil
                }
                completeCustomer(callback, context: context, info: info, error: error)
            }
        case "offerings":
            Purchases.shared.getOfferings { offerings, error in
                if let error {
                    complete(callback, context: context, status: 2, text: error.localizedDescription)
                    return
                }
                guard let offering = offerings?.current else {
                    completeJSON(callback, context: context, object: ["current": NSNull()])
                    return
                }
                let packages: [[String: Any]] = offering.availablePackages.map { package in
                    ["identifier": package.identifier, "product": productJSON(package.storeProduct)]
                }
                completeJSON(
                    callback,
                    context: context,
                    object: ["current": ["identifier": offering.identifier, "packages": packages]]
                )
            }
        case "products":
            guard
                let value,
                let data = value.data(using: .utf8),
                let identifiers = try? JSONDecoder().decode([String].self, from: data)
            else {
                complete(callback, context: context, status: 2, text: "Invalid product IDs")
                return
            }
            Purchases.shared.getProducts(identifiers) { products in
                completeJSON(callback, context: context, object: products.map(productJSON))
            }
        case "purchase":
            guard
                State.appUserID != nil,
                let productID = value,
                !productID.isEmpty
            else {
                complete(callback, context: context, status: 2, text: "A signed-in user and product ID are required")
                return
            }
            Purchases.shared.getProducts([productID]) { products in
                guard let product = products.first(where: { $0.productIdentifier == productID }) else {
                    complete(callback, context: context, status: 2, text: "Product is unavailable in the App Store")
                    return
                }
                let callbackContext = SendableContext(pointer: context)
                Purchases.shared.purchase(product: product) { transaction, info, error, cancelled in
                    if cancelled {
                        complete(callback, context: callbackContext.pointer, status: 1, text: "Purchase was cancelled")
                    } else if let error {
                        complete(callback, context: callbackContext.pointer, status: 2, text: error.localizedDescription)
                    } else if let info {
                        completeJSON(
                            callback,
                            context: callbackContext.pointer,
                            object: [
                                "product_id": productID,
                                "transaction_id": (transaction?.transactionIdentifier as Any?) ?? NSNull(),
                                "customer_info": customerJSON(info)
                            ]
                        )
                    } else {
                        complete(callback, context: callbackContext.pointer, status: 2, text: "RevenueCat returned no purchase information")
                    }
                }
            }
        case "restore":
            Purchases.shared.restorePurchases { info, error in
                completeCustomer(callback, context: context, info: info, error: error)
            }
        case "customer_info":
            Purchases.shared.getCustomerInfo { info, error in
                completeCustomer(callback, context: context, info: info, error: error)
            }
        default:
            complete(callback, context: context, status: 2, text: "Unknown RevenueCat operation")
        }
    }
}
