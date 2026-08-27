#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>
#include <shellapi.h>

#include <memory>
#include <string>

#include "win32_window.h"

inline constexpr ULONG_PTR kLanChatOpenConversationCopyData = 0x4C434F43;

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project,
                         bool start_hidden = false);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  void ShowMainWindow();
  void OpenConversationRoute(const std::string& route);
  void ShowTrayMenu();
  void InvokeDartAction(const std::string& method);

  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
      platform_channel_;
  NOTIFYICONDATAW tray_icon_{};
  bool tray_icon_added_ = false;
  bool close_to_tray_ = true;
  bool start_hidden_ = false;
  std::string notification_route_;
  HRESULT toast_identity_status_ = E_PENDING;
  HANDLE network_change_handle_ = nullptr;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
