#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>
#include <shellapi.h>
#include <shobjidl.h>

#include <algorithm>

#include "flutter_window.h"
#include "shell_identity.h"
#include "utils.h"

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();
  if (FAILED(::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) {
    return EXIT_FAILURE;
  }
  // The shell must know the application identity before any HWND is created.
  ::SetCurrentProcessExplicitAppUserModelID(kAppUserModelId);
  if (!command_line_arguments.empty() && command_line_arguments[0] == "--repair-shell-identity") {
    int argument_count = 0;
    LPWSTR* arguments = ::CommandLineToArgvW(::GetCommandLineW(), &argument_count);
    if (arguments == nullptr || argument_count > 3) {
      ::LocalFree(arguments);
      ::CoUninitialize();
      return EXIT_FAILURE;
    }
    const HRESULT result = EnsureShellIdentity(argument_count == 3 ? arguments[2] : nullptr);
    ::LocalFree(arguments);
    ::CoUninitialize();
    return SUCCEEDED(result) ? EXIT_SUCCESS : EXIT_FAILURE;
  }
  HANDLE single_instance =
      ::CreateMutexW(nullptr, TRUE, L"Local\\LANChat.SingleInstance.V1");
  if (single_instance == nullptr) {
    ::CoUninitialize();
    return EXIT_FAILURE;
  }
  if (::GetLastError() == ERROR_ALREADY_EXISTS) {
    if (HWND existing = ::FindWindowW(nullptr, L"\x732B\x732B\x5FEB\x4F20")) {
      const auto route = std::find_if(
          command_line_arguments.begin(), command_line_arguments.end(),
          [](const std::string& argument) {
            return argument.rfind("openConversation:", 0) == 0;
          });
      if (route != command_line_arguments.end()) {
        COPYDATASTRUCT data{};
        data.dwData = kLanChatOpenConversationCopyData;
        data.cbData = static_cast<DWORD>(route->size() + 1);
        data.lpData = const_cast<char*>(route->c_str());
        DWORD_PTR ignored = 0;
        ::SendMessageTimeoutW(existing, WM_COPYDATA, 0,
                              reinterpret_cast<LPARAM>(&data),
                              SMTO_ABORTIFHUNG, 2000, &ignored);
      }
      ::ShowWindow(existing, SW_RESTORE);
      ::SetForegroundWindow(existing);
    }
    ::CloseHandle(single_instance);
    ::CoUninitialize();
    return EXIT_SUCCESS;
  }
  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  flutter::DartProject project(L"data");

  const bool start_hidden =
      std::find(command_line_arguments.begin(), command_line_arguments.end(),
                "--background") != command_line_arguments.end();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project, start_hidden);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"\x732B\x732B\x5FEB\x4F20", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(false);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  ::ReleaseMutex(single_instance);
  ::CloseHandle(single_instance);
  return EXIT_SUCCESS;
}
