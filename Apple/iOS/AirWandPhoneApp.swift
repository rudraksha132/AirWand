import SwiftUI

@main
struct AirWandPhoneApp: App {
    @StateObject private var session = PointerSession()

    var body: some Scene {
        WindowGroup {
            PhoneRoot()
                .environmentObject(session)
                .preferredColorScheme(.dark)
        }
    }
}
