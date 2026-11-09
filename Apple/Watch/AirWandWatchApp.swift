import SwiftUI

@main
struct AirWandWatchApp: App {
    @StateObject private var session = PointerSession()

    var body: some Scene {
        WindowGroup {
            WatchRoot()
                .environmentObject(session)
        }
    }
}
