#include <windows.h>
#include <propkey.h>
#include <propvarutil.h>
#include <shobjidl.h>
#include <wrl/client.h>

#include <filesystem>
#include <fstream>
#include <iostream>

#include "resource.h"
#include "shell_identity.h"
#include "win32_window.h"

using Microsoft::WRL::ComPtr;

bool MakeLink(const std::filesystem::path& file, const std::filesystem::path& target,
              const wchar_t* app_id, const wchar_t* arguments = L"") {
  ComPtr<IShellLinkW> link;
  ComPtr<IPropertyStore> store;
  ComPtr<IPersistFile> persist;
  if (FAILED(CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&link))) ||
      FAILED(link->SetPath(target.c_str())) || FAILED(link->SetArguments(arguments)) ||
      FAILED(link.As(&store))) return false;
  PROPVARIANT value{};
  if (FAILED(InitPropVariantFromString(app_id, &value))) return false;
  const HRESULT result = store->SetValue(PKEY_AppUserModel_ID, value);
  PropVariantClear(&value);
  return SUCCEEDED(result) && SUCCEEDED(store->Commit()) && SUCCEEDED(link.As(&persist)) &&
      SUCCEEDED(persist->Save(file.c_str(), TRUE));
}

bool CheckLink(const std::filesystem::path& file, const std::filesystem::path& target) {
  ComPtr<IShellLinkW> link;
  ComPtr<IPropertyStore> store;
  ComPtr<IPersistFile> persist;
  if (FAILED(CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&link))) ||
      FAILED(link.As(&persist)) || FAILED(persist->Load(file.c_str(), STGM_READ)) ||
      FAILED(link.As(&store))) return false;
  wchar_t path[MAX_PATH]{};
  wchar_t icon[MAX_PATH]{};
  int icon_index = -1;
  PROPVARIANT value{};
  const HRESULT read = store->GetValue(PKEY_AppUserModel_ID, &value);
  const bool matches = SUCCEEDED(read) && value.vt == VT_LPWSTR &&
      value.pwszVal != nullptr &&
      wcscmp(value.pwszVal, kAppUserModelId) == 0 &&
      SUCCEEDED(link->GetPath(path, MAX_PATH, nullptr, SLGP_RAWPATH)) &&
      SUCCEEDED(link->GetIconLocation(icon, MAX_PATH, &icon_index)) &&
      std::filesystem::path(path) == target && std::filesystem::path(icon) == target && icon_index == 0;
  PropVariantClear(&value);
  return matches;
}

bool CheckWindowIcons() {
  Win32Window window;
  if (!window.Create(L"NekoSend icon regression test", {0, 0}, {640, 520})) return false;
  const HWND child = CreateWindowW(L"STATIC", L"", WS_CHILD, 0, 0, 1, 1,
      window.GetHandle(), nullptr, GetModuleHandle(nullptr), nullptr);
  if (child == nullptr) return false;
  window.SetChildContent(child);
  const UINT dpi = GetDpiForWindow(window.GetHandle());
  for (const bool small_icon : {false, true}) {
    const HICON expected = static_cast<HICON>(LoadImageW(GetModuleHandle(nullptr), MAKEINTRESOURCEW(IDI_APP_ICON),
        IMAGE_ICON, GetSystemMetricsForDpi(small_icon ? SM_CXSMICON : SM_CXICON, dpi),
        GetSystemMetricsForDpi(small_icon ? SM_CYSMICON : SM_CYICON, dpi), LR_SHARED));
    for (const HWND target : {window.GetHandle(), child}) {
      const HICON actual = reinterpret_cast<HICON>(SendMessageW(target, WM_GETICON,
          small_icon ? ICON_SMALL : ICON_BIG, dpi));
      if (expected == nullptr || actual != expected) return false;
    }
  }
  return true;
}

int main() {
  if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) return 1;
  wchar_t temp[MAX_PATH]{};
  if (GetTempPathW(MAX_PATH, temp) == 0) return 1;
  const auto root = std::filesystem::path(temp) /
      (L"NekoSend-shell-test-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64()));
  std::filesystem::create_directory(root);
  const auto executable = root / L"lan_chat.exe";
  std::ofstream(executable).put('\0');
  const auto legacy = root / L"LAN Chat.lnk";
  const auto unrelated = root / L"\x732B\x732B\x5FEB\x4F20.lnk";
  bool passed = MakeLink(legacy, executable, L"dev.lanchat.LANChat") &&
      MakeLink(unrelated, executable, L"some.other.application") &&
      SUCCEEDED(EnsureShellIdentity(executable.c_str(), root.c_str())) &&
      CheckLink(root / kAppShortcutName, executable) && std::filesystem::exists(unrelated);
#ifdef _DEBUG
  passed = passed && std::filesystem::exists(legacy) && wcscmp(kAppUserModelId, L"dev.lanchat.LANChat.Debug") == 0;
#else
  passed = passed && !std::filesystem::exists(legacy) && wcscmp(kAppUserModelId, L"dev.lanchat.LANChat") == 0;
#endif
  passed = passed && SUCCEEDED(EnsureShellIdentity(executable.c_str(), root.c_str())) && CheckWindowIcons();
  passed = passed && MakeLink(legacy, executable, kAppUserModelId, L"--custom-launch") &&
      SUCCEEDED(EnsureShellIdentity(executable.c_str(), root.c_str())) && std::filesystem::exists(legacy);
  std::filesystem::remove_all(root);
  CoUninitialize();
  std::cout << (passed ? "PASS" : "FAIL") << ": taskbar identity, legacy migration, unrelated links, repeat startup and window icons\n";
  return passed ? 0 : 1;
}
