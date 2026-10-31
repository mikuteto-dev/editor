// Makes the app the default for every type it lists in Info.plist.
// usage: swift script/set-default.swift ~/Applications/Editor.app
import AppKit
import UniformTypeIdentifiers

let app = URL(fileURLWithPath: CommandLine.arguments[1])
let plist = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))!
let docs = plist["CFBundleDocumentTypes"] as! [[String: Any]]
let ids = docs
    .filter { ($0["LSHandlerRank"] as? String) == "Default" }
    .flatMap { $0["LSItemContentTypes"] as! [String] }

let done = DispatchGroup()
for id in ids {
    guard let type = UTType(id) else {
        print("unknown type \(id)")
        continue
    }
    done.enter()
    NSWorkspace.shared.setDefaultApplication(at: app, toOpen: type) { error in
        print(id, error.map { "failed: \($0.localizedDescription)" } ?? "ok")
        done.leave()
    }
}
done.notify(queue: .main) { exit(0) }
dispatchMain()
