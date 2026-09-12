import Cocoa
import FlutterMacOS
import CryptoKit
import Network
import ServiceManagement
import UniformTypeIdentifiers
import UserNotifications

@main
class AppDelegate: FlutterAppDelegate, NSWindowDelegate, UNUserNotificationCenterDelegate {
  private var channel: FlutterMethodChannel?
  private var statusItem: NSStatusItem?
  private var clipboardTimer: Timer?
  private var clipboardCount = NSPasteboard.general.changeCount
  private var suppressedImageCount: Int?
  private var closeToTray = true
  private var notificationsEnabled = false
  private var exiting = false
  private var activity: NSObjectProtocol?
  private let networkMonitor = NWPathMonitor()

  override func applicationDidFinishLaunching(_ notification: Notification) {
    guard let window = mainFlutterWindow,
          let controller = window.contentViewController as? FlutterViewController else { return }
    window.delegate = self
    window.minSize = NSSize(width: 760, height: 560)
    channel = FlutterMethodChannel(name: "dev.lanchat/platform", binaryMessenger: controller.engine.binaryMessenger)
    channel?.setMethodCallHandler { [weak self] call, result in
      guard let self = self else { return }
      do { try self.handle(call, result: result) }
      catch { result(FlutterError(code: "MACOS_PLATFORM", message: error.localizedDescription, details: nil)) }
    }
    UNUserNotificationCenter.current().delegate = self
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    let trayImage = NSImage(named: "AppIcon")?.copy() as? NSImage
    trayImage?.size = NSSize(width: 18, height: 18)
    item.button?.image = trayImage
    let menu = NSMenu()
    for (title, action) in [("Open NekoSend", #selector(showWindow)),
                            ("Send Clipboard", #selector(sendClipboard)),
                            ("Pause Transfers", #selector(pauseTransfers)),
                            ("Quit NekoSend", #selector(requestQuit))] {
      let entry = NSMenuItem(title: title, action: action, keyEquivalent: "")
      entry.target = self
      menu.addItem(entry)
    }
    item.menu = menu
    statusItem = item
    clipboardTimer = Timer.scheduledTimer(withTimeInterval: 0.75, repeats: true) { [weak self] _ in
      guard let self = self else { return }
      let count = NSPasteboard.general.changeCount
      if count != self.clipboardCount {
        self.clipboardCount = count
        self.channel?.invokeMethod("clipboardChanged", arguments: nil)
      }
    }
    networkMonitor.pathUpdateHandler = { [weak self] _ in
      DispatchQueue.main.async { self?.channel?.invokeMethod("networkChanged", arguments: nil) }
    }
    networkMonitor.start(queue: DispatchQueue(label: "NekoSend.network"))
  }

  @objc private func showWindow() {
    mainFlutterWindow?.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }
  @objc private func sendClipboard() { channel?.invokeMethod("traySendClipboard", arguments: nil) }
  @objc private func pauseTransfers() { channel?.invokeMethod("trayPauseAllTransfers", arguments: nil) }
  @objc private func requestQuit() { channel?.invokeMethod("trayExitRequested", arguments: nil) }

  func windowShouldClose(_ sender: NSWindow) -> Bool {
    if closeToTray { sender.orderOut(nil) }
    else { channel?.invokeMethod("windowExitRequested", arguments: nil) }
    return false
  }

  override func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    if exiting { return .terminateNow }
    requestQuit()
    return .terminateCancel
  }

  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    showWindow()
    return true
  }

  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  private func directory(_ kind: FileManager.SearchPathDirectory) throws -> URL {
    let base = try FileManager.default.url(for: kind, in: .userDomainMask, appropriateFor: nil, create: true)
    let url = base.appendingPathComponent("NekoSend", isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) throws {
    let args = call.arguments as? [String: Any] ?? [:]
    switch call.method {
    case "getBootstrapInfo":
      result(["dataDirectory": try directory(.applicationSupportDirectory).path,
              "deviceName": Host.current().localizedName ?? ProcessInfo.processInfo.hostName])
    case "getDefaultReceiveDirectory": result(try directory(.downloadsDirectory).path)
    case "getSavedReceiveTree": result(nil)
    case "pickSource", "pickReceiveDirectory":
      let panel = NSOpenPanel()
      let folder = call.method == "pickReceiveDirectory" || args["kind"] as? String == "folder"
      panel.canChooseDirectories = folder
      panel.canChooseFiles = !folder
      panel.allowsMultipleSelection = false
      panel.canCreateDirectories = folder
      if args["kind"] as? String == "image" { panel.allowedContentTypes = [.image] }
      panel.begin { response in
        guard response == .OK, let path = panel.url?.path else { result(nil); return }
        if call.method == "pickReceiveDirectory" { result(["treeUri": path]) }
        else { result(path) }
      }
    case "openReference":
      guard let reference = args["reference"] as? String else { throw CocoaError(.fileReadInvalidFileName) }
      let url = reference.hasPrefix("file://") ? URL(string: reference)! : URL(fileURLWithPath: reference)
      if args["showInFolder"] as? Bool == true { NSWorkspace.shared.activateFileViewerSelecting([url]) }
      else if !NSWorkspace.shared.open(url) { throw CocoaError(.fileReadUnknown) }
      result(nil)
    case "readClipboardContent":
      let board = NSPasteboard.general
      if let urls = board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], !urls.isEmpty {
        guard urls.count <= 10000 else { throw CocoaError(.fileReadTooLarge) }
        result(["items": urls.map { ["sourceRef": $0.path, "displayName": $0.lastPathComponent] }])
      } else if board.availableType(from: [.png, .tiff]) != nil {
        let sequence = board.changeCount
        let png = board.data(forType: .png)
        let tiff = png == nil ? board.data(forType: .tiff) : nil
        DispatchQueue.global(qos: .userInitiated).async {
          do {
            guard var image = try self.cacheClipboardImage(png: png, tiff: tiff, suppressed: false) else {
              throw CocoaError(.fileReadCorruptFile)
            }
            image["kind"] = "image"
            image["ephemeral"] = true
            DispatchQueue.main.async {
              guard board.changeCount == sequence else {
                result(FlutterError(code: "CLIPBOARD_CHANGED", message: "Clipboard changed; paste again", details: nil)); return
              }
              result(["items": [image]])
            }
          } catch {
            DispatchQueue.main.async { result(FlutterError(code: "CLIPBOARD_READ_FAILED", message: error.localizedDescription, details: nil)) }
          }
        }
      } else { result(["text": board.string(forType: .string) ?? ""]) }
    case "readClipboardImage": result(try readClipboardImage())
    case "writeClipboardImage":
      guard let reference = args["reference"] as? String else { throw CocoaError(.fileReadInvalidFileName) }
      let data = try Data(contentsOf: URL(fileURLWithPath: reference))
      guard data.count <= 20 * 1024 * 1024, let image = NSImage(data: data),
            let tiff = image.tiffRepresentation,
            let bitmap = NSBitmapImageRep(data: tiff),
            let png = bitmap.representation(using: .png, properties: [:]) else { throw CocoaError(.fileReadCorruptFile) }
      NSPasteboard.general.clearContents()
      guard NSPasteboard.general.setData(png, forType: .png) else { throw CocoaError(.fileWriteUnknown) }
      suppressedImageCount = NSPasteboard.general.changeCount
      result(nil)
    case "applyAppSettings":
      let startOnBoot = args["startOnBoot"] as? Bool ?? false
      if #available(macOS 13.0, *) {
        let service = SMAppService.mainApp
        if startOnBoot && service.status == .notRegistered { try service.register() }
        else if !startOnBoot && (service.status == .enabled || service.status == .requiresApproval) { try service.unregister() }
      } else if startOnBoot {
        result(FlutterError(code: "UNSUPPORTED", message: "Login at startup requires macOS 13 or later", details: nil)); return
      }
      closeToTray = args["closeToTray"] as? Bool ?? true
      let enableNotifications = args["notificationsEnabled"] as? Bool ?? false
      if enableNotifications && !notificationsEnabled {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
      }
      notificationsEnabled = enableNotifications
      result(nil)
    case "updateActiveTransferCount":
      let count = args["count"] as? Int ?? 0
      if count > 0 && activity == nil {
        activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiatedAllowingIdleSystemSleep], reason: "NekoSend file transfer")
      } else if count == 0, let token = activity {
        ProcessInfo.processInfo.endActivity(token)
        activity = nil
      }
      result(nil)
    case "showNotification":
      guard notificationsEnabled else { result(nil); return }
      let content = UNMutableNotificationContent()
      content.title = args["title"] as? String ?? "NekoSend"
      content.body = args["body"] as? String ?? ""
      content.sound = .default
      if let conversation = args["conversationId"] as? String { content.userInfo["conversationId"] = conversation }
      UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)) { error in
        DispatchQueue.main.async {
          if let error = error { result(FlutterError(code: "NOTIFICATION", message: error.localizedDescription, details: nil)) }
          else { result(nil) }
        }
      }
    case "exitApplication":
      exiting = true
      clipboardTimer?.invalidate()
      networkMonitor.cancel()
      if let token = activity { ProcessInfo.processInfo.endActivity(token) }
      result(nil)
      NSApp.terminate(nil)
    default: result(FlutterMethodNotImplemented)
    }
  }

  private func readClipboardImage() throws -> [String: Any]? {
    let pasteboard = NSPasteboard.general
    return try cacheClipboardImage(png: pasteboard.data(forType: .png), tiff: pasteboard.data(forType: .tiff),
                                  suppressed: suppressedImageCount == pasteboard.changeCount)
  }

  private func cacheClipboardImage(png original: Data?, tiff: Data?, suppressed: Bool) throws -> [String: Any]? {
    var png = original
    if png == nil, let tiff = tiff, let bitmap = NSBitmapImageRep(data: tiff) {
      guard bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0, bitmap.pixelsWide * bitmap.pixelsHigh <= 32 * 1024 * 1024 else { throw CocoaError(.fileReadTooLarge) }
      png = bitmap.representation(using: .png, properties: [:])
    }
    guard let data = png else { return nil }
    guard !data.isEmpty && data.count <= 20 * 1024 * 1024 else { throw CocoaError(.fileReadTooLarge) }
    let hash = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    let name = "clipboard-\(hash).png"
    let url = try directory(.cachesDirectory).appendingPathComponent(name)
    try data.write(to: url, options: .atomic)
    let modified = try url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate ?? Date()
    return ["displayName": name, "relativePath": name, "sourceRef": url.path,
            "size": data.count, "modifiedAtMs": Int(modified.timeIntervalSince1970 * 1000),
            "fingerprint": hash, "suppressSync": suppressed]
  }

  func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                              withCompletionHandler completionHandler: @escaping () -> Void) {
    DispatchQueue.main.async {
      self.showWindow()
      if let conversation = response.notification.request.content.userInfo["conversationId"] as? String {
        self.channel?.invokeMethod("notificationOpenConversation", arguments: conversation)
      }
      completionHandler()
    }
  }
  func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                              withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
    completionHandler([.banner, .sound])
  }
}
