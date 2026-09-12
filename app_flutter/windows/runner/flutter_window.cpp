#include <winsock2.h>

#include "flutter_window.h"

#include <flutter/standard_method_codec.h>
#include <bcrypt.h>
#include <netioapi.h>
#include <propkey.h>
#include <propvarutil.h>
#include <shobjidl.h>
#include <shlobj.h>
#include <shellapi.h>
#include <wincodec.h>
#include <winrt/Windows.Data.Xml.Dom.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Notifications.h>
#include <winrt/base.h>

#include <algorithm>
#include <array>
#include <chrono>
#include <climits>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <new>
#include <optional>
#include <sstream>
#include <vector>
#include <thread>
#include <wrl/client.h>

#include "flutter/generated_plugin_registrant.h"
#include "resource.h"
#include "shell_identity.h"
#include "utils.h"

namespace {

constexpr UINT kTrayCallbackMessage = WM_APP + 42;
constexpr UINT kNetworkChangedMessage = WM_APP + 43;
constexpr UINT kToastActivatedMessage = WM_APP + 44;
constexpr UINT kClipboardReadComplete = WM_APP + 45;
constexpr UINT kTrayOpen = 41001;
constexpr UINT kTraySendClipboard = 41002;
constexpr UINT kTrayPauseAll = 41003;
constexpr UINT kTrayExit = 41004;
constexpr size_t kMaxClipboardImageBytes = 20 * 1024 * 1024;
constexpr wchar_t kClipboardOriginFormat[] = L"LAN_CHAT_ORIGIN";

std::wstring Utf16FromUtf8(const std::string& value);

std::wstring EscapeToastXml(const std::wstring& value) {
  std::wstring escaped;
  escaped.reserve(value.size());
  for (const wchar_t character : value) {
    switch (character) {
      case L'&':
        escaped += L"&amp;";
        break;
      case L'<':
        escaped += L"&lt;";
        break;
      case L'>':
        escaped += L"&gt;";
        break;
      case L'\"':
        escaped += L"&quot;";
        break;
      case L'\'':
        escaped += L"&apos;";
        break;
      default:
        escaped += character;
        break;
    }
  }
  return escaped;
}

HRESULT ShowWindowsToast(HWND window, const std::wstring& title,
                         const std::wstring& body,
                         const std::string& conversation_route) {
  try {
    const std::wstring route =
        L"openConversation:" + Utf16FromUtf8(conversation_route);
    const std::wstring escaped_route = EscapeToastXml(route);
    std::wstring xml =
        L"<toast launch=\"" + escaped_route +
        L"\"><visual><binding template=\"ToastGeneric\"><text>" +
        EscapeToastXml(title) + L"</text><text>" + EscapeToastXml(body) +
        L"</text></binding></visual>";
    if (!conversation_route.empty()) {
      xml += L"<actions><action content=\"\x6253\x5F00\" arguments=\"" +
             escaped_route + L"\" activationType=\"foreground\"/></actions>";
    }
    xml += L"</toast>";

    winrt::Windows::Data::Xml::Dom::XmlDocument document;
    document.LoadXml(xml);
    winrt::Windows::UI::Notifications::ToastNotification toast(document);
    toast.Activated(
        [window](const auto&, const winrt::Windows::Foundation::IInspectable& raw) {
          std::string route;
          if (raw) {
            const auto arguments =
                raw.try_as<winrt::Windows::UI::Notifications::ToastActivatedEventArgs>();
            if (arguments) route = winrt::to_string(arguments.Arguments());
          }
          auto* posted_route = new (std::nothrow) std::string(std::move(route));
          if (posted_route == nullptr ||
              !::PostMessageW(window, kToastActivatedMessage, 0,
                              reinterpret_cast<LPARAM>(posted_route))) {
            delete posted_route;
          }
        });
    winrt::Windows::UI::Notifications::ToastNotificationManager::
        CreateToastNotifier(kAppUserModelId)
            .Show(toast);
    return S_OK;
  } catch (const winrt::hresult_error& error) {
    return error.code();
  } catch (...) {
    return E_FAIL;
  }
}

VOID CALLBACK NetworkChangedCallback(PVOID context,
                                      PMIB_IPINTERFACE_ROW /*row*/,
                                      MIB_NOTIFICATION_TYPE /*type*/) {
  const HWND window = static_cast<HWND>(context);
  if (::IsWindow(window)) {
    ::PostMessageW(window, kNetworkChangedMessage, 0, 0);
  }
}

std::optional<std::string> Sha256Hex(const std::vector<BYTE>& bytes) {
  BCRYPT_ALG_HANDLE algorithm = nullptr;
  NTSTATUS status = ::BCryptOpenAlgorithmProvider(
      &algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
  std::array<BYTE, 32> hash{};
  if (BCRYPT_SUCCESS(status)) {
    status = ::BCryptHash(algorithm, nullptr, 0,
                          const_cast<PUCHAR>(bytes.data()),
                          static_cast<ULONG>(bytes.size()), hash.data(),
                          static_cast<ULONG>(hash.size()));
  }
  if (algorithm != nullptr) {
    ::BCryptCloseAlgorithmProvider(algorithm, 0);
  }
  if (!BCRYPT_SUCCESS(status)) return std::nullopt;
  std::ostringstream output;
  output << std::hex << std::setfill('0');
  for (BYTE byte : hash) output << std::setw(2) << static_cast<int>(byte);
  return output.str();
}

HRESULT SetStartAtLogin(bool enabled) {
  HKEY key = nullptr;
  LONG status = ::RegCreateKeyExW(
      HKEY_CURRENT_USER,
      L"Software\\Microsoft\\Windows\\CurrentVersion\\Run", 0, nullptr,
      REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, nullptr, &key, nullptr);
  if (status != ERROR_SUCCESS) {
    return HRESULT_FROM_WIN32(status);
  }
  constexpr wchar_t kValueName[] = L"LAN Chat";
  if (!enabled) {
    status = ::RegDeleteValueW(key, kValueName);
    ::RegCloseKey(key);
    return status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND
               ? S_OK
               : HRESULT_FROM_WIN32(status);
  }
  std::vector<wchar_t> path(32768);
  const DWORD length = ::GetModuleFileNameW(
      nullptr, path.data(), static_cast<DWORD>(path.size()));
  if (length == 0 || length >= path.size()) {
    const DWORD error = ::GetLastError();
    ::RegCloseKey(key);
    return HRESULT_FROM_WIN32(error == ERROR_SUCCESS ? ERROR_BAD_PATHNAME
                                                     : error);
  }
  const std::wstring command = L"\"" + std::wstring(path.data(), length) +
                               L"\" --background";
  status = ::RegSetValueExW(
      key, kValueName, 0, REG_SZ,
      reinterpret_cast<const BYTE*>(command.c_str()),
      static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
  ::RegCloseKey(key);
  return status == ERROR_SUCCESS ? S_OK : HRESULT_FROM_WIN32(status);
}

std::optional<std::string> DefaultReceiveDirectory(HRESULT* result_code) {
  PWSTR downloads = nullptr;
  HRESULT result = ::SHGetKnownFolderPath(FOLDERID_Downloads, KF_FLAG_CREATE,
                                          nullptr, &downloads);
  if (FAILED(result)) {
    *result_code = result;
    return std::nullopt;
  }
  const std::filesystem::path path =
      std::filesystem::path(downloads) / L"LAN Chat";
  ::CoTaskMemFree(downloads);
  std::error_code error;
  std::filesystem::create_directories(path, error);
  if (error) {
    *result_code = HRESULT_FROM_WIN32(error.value());
    return std::nullopt;
  }
  *result_code = S_OK;
  return Utf8FromUtf16(path.wstring().c_str());
}

struct ClipboardImageSource {
  std::string path;
  int64_t size;
  int64_t modified_at_ms;
  std::string fingerprint;
  bool suppress_sync;
};

HRESULT EncodeClipboardPng(const std::vector<BYTE>& bitmap, const std::filesystem::path& path) {
  using Microsoft::WRL::ComPtr;
  ComPtr<IStream> input;
  ComPtr<IWICImagingFactory> factory;
  ComPtr<IWICBitmapDecoder> decoder;
  ComPtr<IWICBitmapFrameDecode> source;
  ComPtr<IWICStream> output;
  ComPtr<IWICBitmapEncoder> encoder;
  ComPtr<IWICBitmapFrameEncode> frame;
  HRESULT code = CreateStreamOnHGlobal(nullptr, TRUE, &input);
  if (SUCCEEDED(code)) code = input->Write(bitmap.data(), static_cast<ULONG>(bitmap.size()), nullptr);
  if (SUCCEEDED(code)) code = input->Seek({}, STREAM_SEEK_SET, nullptr);
  if (SUCCEEDED(code)) code = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&factory));
  if (SUCCEEDED(code)) code = factory->CreateDecoderFromStream(input.Get(), nullptr, WICDecodeMetadataCacheOnLoad, &decoder);
  if (SUCCEEDED(code)) code = decoder->GetFrame(0, &source);
  UINT width = 0, height = 0;
  if (SUCCEEDED(code)) code = source->GetSize(&width, &height);
  if (SUCCEEDED(code) && (width == 0 || height == 0 || static_cast<uint64_t>(width) * height > 32 * 1024 * 1024)) return E_INVALIDARG;
  if (SUCCEEDED(code)) code = factory->CreateStream(&output);
  if (SUCCEEDED(code)) code = output->InitializeFromFilename(path.c_str(), GENERIC_WRITE);
  if (SUCCEEDED(code)) code = factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder);
  if (SUCCEEDED(code)) code = encoder->Initialize(output.Get(), WICBitmapEncoderNoCache);
  if (SUCCEEDED(code)) code = encoder->CreateNewFrame(&frame, nullptr);
  if (SUCCEEDED(code)) code = frame->Initialize(nullptr);
  if (SUCCEEDED(code)) code = frame->SetSize(width, height);
  WICPixelFormatGUID pixel_format = GUID_WICPixelFormat32bppBGRA;
  if (SUCCEEDED(code)) code = frame->SetPixelFormat(&pixel_format);
  if (SUCCEEDED(code)) code = frame->WriteSource(source.Get(), nullptr);
  if (SUCCEEDED(code)) code = frame->Commit();
  if (SUCCEEDED(code)) code = encoder->Commit();
  return code;
}

std::wstring Utf16FromUtf8(const std::string& value) {
  if (value.empty()) return std::wstring();
  const int length = ::MultiByteToWideChar(
      CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
      static_cast<int>(value.size()), nullptr, 0);
  if (length <= 0) return std::wstring();
  std::wstring result(static_cast<size_t>(length), L'\0');
  if (::MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
                            static_cast<int>(value.size()), result.data(),
                            length) != length) {
    return std::wstring();
  }
  return result;
}

std::optional<ClipboardImageSource> ReadClipboardImage(HWND owner,
                                                       HRESULT* result_code) {
  if (!::OpenClipboard(owner)) {
    *result_code = HRESULT_FROM_WIN32(::GetLastError());
    return std::nullopt;
  }
  const UINT origin_format = ::RegisterClipboardFormatW(kClipboardOriginFormat);
  const bool suppress_sync =
      origin_format != 0 && ::IsClipboardFormatAvailable(origin_format);
  const UINT format = ::IsClipboardFormatAvailable(CF_DIBV5) ? CF_DIBV5
                                                             : CF_DIB;
  if (!::IsClipboardFormatAvailable(format)) {
    ::CloseClipboard();
    *result_code = S_FALSE;
    return std::nullopt;
  }
  HGLOBAL handle = static_cast<HGLOBAL>(::GetClipboardData(format));
  const SIZE_T dib_size = handle == nullptr ? 0 : ::GlobalSize(handle);
  const auto* dib = handle == nullptr
                        ? nullptr
                        : static_cast<const BYTE*>(::GlobalLock(handle));
  if (dib == nullptr || dib_size < sizeof(BITMAPINFOHEADER) ||
      dib_size + sizeof(BITMAPFILEHEADER) > kMaxClipboardImageBytes) {
    if (dib != nullptr) {
      ::GlobalUnlock(handle);
    }
    ::CloseClipboard();
    *result_code = HRESULT_FROM_WIN32(ERROR_FILE_TOO_LARGE);
    return std::nullopt;
  }
  const auto* info = reinterpret_cast<const BITMAPINFOHEADER*>(dib);
  if (info->biSize < sizeof(BITMAPINFOHEADER) || info->biSize > dib_size) {
    ::GlobalUnlock(handle);
    ::CloseClipboard();
    *result_code = E_INVALIDARG;
    return std::nullopt;
  }
  DWORD color_count = info->biClrUsed;
  if (color_count == 0 && info->biBitCount <= 8) {
    color_count = 1u << info->biBitCount;
  }
  DWORD masks_size = 0;
  if (info->biSize == sizeof(BITMAPINFOHEADER)) {
    if (info->biCompression == BI_BITFIELDS) {
      masks_size = 3 * sizeof(DWORD);
    }
  }
  const uint64_t pixel_offset = sizeof(BITMAPFILEHEADER) + info->biSize +
                                masks_size +
                                static_cast<uint64_t>(color_count) *
                                    sizeof(RGBQUAD);
  if (pixel_offset > sizeof(BITMAPFILEHEADER) + dib_size) {
    ::GlobalUnlock(handle);
    ::CloseClipboard();
    *result_code = E_INVALIDARG;
    return std::nullopt;
  }

  wchar_t local_app_data[MAX_PATH]{};
  const DWORD length = ::GetEnvironmentVariableW(
      L"LOCALAPPDATA", local_app_data, static_cast<DWORD>(MAX_PATH));
  if (length == 0 || length >= MAX_PATH) {
    ::GlobalUnlock(handle);
    ::CloseClipboard();
    *result_code = HRESULT_FROM_WIN32(ERROR_PATH_NOT_FOUND);
    return std::nullopt;
  }
  std::filesystem::path directory =
      std::filesystem::path(local_app_data) / L"LAN Chat" /
      L"clipboard-sources";
  std::error_code error;
  std::filesystem::create_directories(directory, error);
  if (error) {
    ::GlobalUnlock(handle);
    ::CloseClipboard();
    *result_code = HRESULT_FROM_WIN32(error.value());
    return std::nullopt;
  }
  GUID identifier{};
  CoCreateGuid(&identifier);
  wchar_t identifier_text[40]{};
  StringFromGUID2(identifier, identifier_text, 40);
  const std::filesystem::path path = directory / (std::wstring(L"clipboard-") + identifier_text + L".png");
  BITMAPFILEHEADER file_header{};
  file_header.bfType = 0x4D42;
  file_header.bfSize = static_cast<DWORD>(sizeof(file_header) + dib_size);
  file_header.bfOffBits = static_cast<DWORD>(pixel_offset);
  std::vector<BYTE> encoded(sizeof(file_header) + dib_size);
  std::copy_n(reinterpret_cast<const BYTE*>(&file_header), sizeof(file_header),
              encoded.data());
  std::copy_n(dib, dib_size, encoded.data() + sizeof(file_header));
  ::GlobalUnlock(handle);
  ::CloseClipboard();
  const HRESULT encoded_result = EncodeClipboardPng(encoded, path);
  const auto png_size = std::filesystem::file_size(path, error);
  if (FAILED(encoded_result) || error || png_size > kMaxClipboardImageBytes) {
    std::filesystem::remove(path, error);
    *result_code = E_FAIL;
    return std::nullopt;
  }
  std::ifstream input(path, std::ios::binary);
  std::vector<BYTE> png(static_cast<size_t>(png_size));
  input.read(reinterpret_cast<char*>(png.data()), static_cast<std::streamsize>(png.size()));
  const auto fingerprint = input.good() ? Sha256Hex(png) : std::nullopt;
  if (!fingerprint) { *result_code = E_FAIL; return std::nullopt; }
  WIN32_FILE_ATTRIBUTE_DATA attributes{};
  if (!GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &attributes)) { *result_code = E_FAIL; return std::nullopt; }
  ULARGE_INTEGER modified{};
  modified.LowPart = attributes.ftLastWriteTime.dwLowDateTime;
  modified.HighPart = attributes.ftLastWriteTime.dwHighDateTime;
  const int64_t modified_at = static_cast<int64_t>(modified.QuadPart / 10000ULL) - 11644473600000LL;
  *result_code = S_OK;
  return ClipboardImageSource{Utf8FromUtf16(path.wstring().c_str()),
                              static_cast<int64_t>(png_size),
                              modified_at, *fingerprint, suppress_sync};
}

struct ClipboardReadReply {
  std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result;
  flutter::EncodableMap value;
  std::string error;
};

void ReadComposerClipboard(HWND owner, ClipboardReadReply* reply) {
  using flutter::EncodableValue;
  if (!OpenClipboard(owner)) { reply->error = "剪贴板正被占用，请重试"; return; }
  const DWORD sequence = GetClipboardSequenceNumber();
  if (IsClipboardFormatAvailable(CF_HDROP)) {
    const HDROP drop = static_cast<HDROP>(GetClipboardData(CF_HDROP));
    const UINT count = drop == nullptr ? 0 : DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
    flutter::EncodableList items;
    size_t bytes = 0;
    if (count == 0 || count > 10000) reply->error = "剪贴板文件列表无效或过大";
    for (UINT i = 0; reply->error.empty() && i < count; ++i) {
      const UINT length = DragQueryFileW(drop, i, nullptr, 0);
      if (length == 0 || length > 32767) { reply->error = "文件路径无效"; break; }
      std::vector<wchar_t> path(length + 1);
      DragQueryFileW(drop, i, path.data(), length + 1);
      const auto reference = Utf8FromUtf16(path.data());
      if (reference.empty()) { reply->error = "Invalid Unicode file path"; break; }
      bytes += reference.size();
      if (bytes > 16 * 1024 * 1024) { reply->error = "剪贴板文件列表过大"; break; }
      items.emplace_back(flutter::EncodableMap{
        {EncodableValue("sourceRef"), EncodableValue(reference)},
        {EncodableValue("displayName"), EncodableValue(Utf8FromUtf16(std::filesystem::path(path.data()).filename().c_str()))},
      });
    }
    CloseClipboard();
    reply->value[EncodableValue("items")] = EncodableValue(items);
    return;
  }
  const bool image = IsClipboardFormatAvailable(CF_DIB) || IsClipboardFormatAvailable(CF_DIBV5);
  if (!image && IsClipboardFormatAvailable(CF_UNICODETEXT)) {
    const HGLOBAL handle = static_cast<HGLOBAL>(GetClipboardData(CF_UNICODETEXT));
    const auto* text = handle == nullptr ? nullptr : static_cast<const wchar_t*>(GlobalLock(handle));
    if (text != nullptr) {
      const size_t maximum = GlobalSize(handle) / sizeof(wchar_t);
      const size_t length = wcsnlen_s(text, maximum);
      if (length > 20000 || length == maximum) reply->error = "剪贴板文字过长或无效";
      else reply->value[EncodableValue("text")] = EncodableValue(Utf8FromUtf16(text));
      GlobalUnlock(handle);
    }
  }
  CloseClipboard();
  if (image) {
    HRESULT code;
    const auto source = ReadClipboardImage(owner, &code);
    if (!source || sequence != GetClipboardSequenceNumber()) {
      reply->error = "剪贴板图片不可读取或已变化，请重试";
      return;
    }
    reply->value[EncodableValue("items")] = EncodableValue(flutter::EncodableList{EncodableValue(flutter::EncodableMap{
      {EncodableValue("sourceRef"), EncodableValue(source->path)},
      {EncodableValue("displayName"), EncodableValue("剪贴板图片.png")},
      {EncodableValue("kind"), EncodableValue("image")},
      {EncodableValue("ephemeral"), EncodableValue(true)},
      {EncodableValue("fingerprint"), EncodableValue(source->fingerprint)},
    })});
  }
}

HRESULT WriteClipboardImage(HWND owner, const std::wstring& path,
                            const std::string& metadata) {
  IWICImagingFactory* factory = nullptr;
  IWICBitmapDecoder* decoder = nullptr;
  IWICBitmapFrameDecode* frame = nullptr;
  IWICFormatConverter* converter = nullptr;
  HRESULT result = ::CoCreateInstance(CLSID_WICImagingFactory, nullptr,
                                      CLSCTX_INPROC_SERVER,
                                      IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) {
    result = factory->CreateDecoderFromFilename(
        path.c_str(), nullptr, GENERIC_READ, WICDecodeMetadataCacheOnLoad,
        &decoder);
  }
  if (SUCCEEDED(result)) {
    result = decoder->GetFrame(0, &frame);
  }
  if (SUCCEEDED(result)) {
    result = factory->CreateFormatConverter(&converter);
  }
  if (SUCCEEDED(result)) {
    result = converter->Initialize(
        frame, GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, nullptr,
        0.0, WICBitmapPaletteTypeCustom);
  }
  UINT width = 0;
  UINT height = 0;
  if (SUCCEEDED(result)) {
    result = converter->GetSize(&width, &height);
  }
  const uint64_t stride = static_cast<uint64_t>(width) * 4;
  const uint64_t pixel_bytes = stride * height;
  if (SUCCEEDED(result) &&
      (width == 0 || height == 0 || pixel_bytes > kMaxClipboardImageBytes ||
       pixel_bytes > UINT_MAX)) {
    result = HRESULT_FROM_WIN32(ERROR_FILE_TOO_LARGE);
  }
  std::vector<BYTE> top_down;
  if (SUCCEEDED(result)) {
    top_down.resize(static_cast<size_t>(pixel_bytes));
    result = converter->CopyPixels(nullptr, static_cast<UINT>(stride),
                                   static_cast<UINT>(pixel_bytes),
                                   top_down.data());
  }
  HGLOBAL clipboard_data = nullptr;
  HGLOBAL origin_data = nullptr;
  if (SUCCEEDED(result)) {
    clipboard_data = ::GlobalAlloc(
        GMEM_MOVEABLE, sizeof(BITMAPV5HEADER) + static_cast<SIZE_T>(pixel_bytes));
    if (clipboard_data == nullptr) {
      result = E_OUTOFMEMORY;
    }
  }
  if (SUCCEEDED(result)) {
    origin_data = ::GlobalAlloc(GMEM_MOVEABLE, metadata.size() + 1);
    auto* memory = origin_data == nullptr
                       ? nullptr
                       : static_cast<char*>(::GlobalLock(origin_data));
    if (memory == nullptr) {
      result = E_OUTOFMEMORY;
    } else {
      std::copy(metadata.begin(), metadata.end(), memory);
      memory[metadata.size()] = '\0';
      ::GlobalUnlock(origin_data);
    }
  }
  if (SUCCEEDED(result)) {
    auto* memory = static_cast<BYTE*>(::GlobalLock(clipboard_data));
    if (memory == nullptr) {
      result = E_OUTOFMEMORY;
    } else {
      auto* header = reinterpret_cast<BITMAPV5HEADER*>(memory);
      *header = {};
      header->bV5Size = sizeof(BITMAPV5HEADER);
      header->bV5Width = static_cast<LONG>(width);
      header->bV5Height = static_cast<LONG>(height);
      header->bV5Planes = 1;
      header->bV5BitCount = 32;
      header->bV5Compression = BI_BITFIELDS;
      header->bV5SizeImage = static_cast<DWORD>(pixel_bytes);
      header->bV5RedMask = 0x00FF0000;
      header->bV5GreenMask = 0x0000FF00;
      header->bV5BlueMask = 0x000000FF;
      header->bV5AlphaMask = 0xFF000000;
      header->bV5CSType = LCS_sRGB;
      BYTE* pixels = memory + sizeof(BITMAPV5HEADER);
      for (UINT row = 0; row < height; ++row) {
        const size_t source_offset = static_cast<size_t>(row * stride);
        const size_t target_offset =
            static_cast<size_t>((height - row - 1) * stride);
        std::copy_n(top_down.data() + source_offset,
                    static_cast<size_t>(stride), pixels + target_offset);
      }
      ::GlobalUnlock(clipboard_data);
    }
  }
  if (SUCCEEDED(result)) {
    if (!::OpenClipboard(owner)) {
      result = HRESULT_FROM_WIN32(::GetLastError());
    } else {
      const UINT origin_format = ::RegisterClipboardFormatW(kClipboardOriginFormat);
      if (!::EmptyClipboard() || origin_format == 0 ||
          ::SetClipboardData(CF_DIBV5, clipboard_data) == nullptr) {
        result = HRESULT_FROM_WIN32(::GetLastError());
      } else {
        clipboard_data = nullptr;
        if (::SetClipboardData(origin_format, origin_data) == nullptr) {
          result = HRESULT_FROM_WIN32(::GetLastError());
        } else {
          origin_data = nullptr;
        }
      }
      ::CloseClipboard();
    }
  }
  if (clipboard_data != nullptr) {
    ::GlobalFree(clipboard_data);
  }
  if (origin_data != nullptr) {
    ::GlobalFree(origin_data);
  }
  if (converter != nullptr) converter->Release();
  if (frame != nullptr) frame->Release();
  if (decoder != nullptr) decoder->Release();
  if (factory != nullptr) factory->Release();
  return result;
}

std::optional<std::string> PickSource(HWND owner, bool folder,
                                      HRESULT* result_code) {
  IFileOpenDialog* dialog = nullptr;
  HRESULT result = ::CoCreateInstance(CLSID_FileOpenDialog, nullptr,
                                      CLSCTX_INPROC_SERVER,
                                      IID_PPV_ARGS(&dialog));
  if (FAILED(result)) {
    *result_code = result;
    return std::nullopt;
  }
  DWORD options = 0;
  result = dialog->GetOptions(&options);
  if (SUCCEEDED(result)) {
    options |= FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
    if (folder) {
      options |= FOS_PICKFOLDERS;
    } else {
      options |= FOS_FILEMUSTEXIST;
    }
    result = dialog->SetOptions(options);
  }
  if (SUCCEEDED(result)) {
    result = dialog->Show(owner);
  }
  IShellItem* item = nullptr;
  if (SUCCEEDED(result)) {
    result = dialog->GetResult(&item);
  }
  PWSTR path = nullptr;
  if (SUCCEEDED(result)) {
    result = item->GetDisplayName(SIGDN_FILESYSPATH, &path);
  }
  std::optional<std::string> selected;
  if (SUCCEEDED(result) && path != nullptr) {
    selected = Utf8FromUtf16(path);
  }
  if (path != nullptr) {
    ::CoTaskMemFree(path);
  }
  if (item != nullptr) {
    item->Release();
  }
  dialog->Release();
  *result_code = result;
  return selected;
}

HRESULT OpenLocalReference(HWND owner, const std::string& reference,
                           bool show_in_folder) {
  if (reference.empty()) return E_INVALIDARG;
  const std::filesystem::path path = std::filesystem::u8path(reference);
  std::error_code status_error;
  if (!std::filesystem::exists(path, status_error)) {
    return HRESULT_FROM_WIN32(status_error ? status_error.value()
                                           : ERROR_FILE_NOT_FOUND);
  }
  const std::wstring path_value = path.wstring();
  if (show_in_folder) {
    const std::wstring parameters = L"/select,\"" + path_value + L"\"";
    SHELLEXECUTEINFOW operation{};
    operation.cbSize = sizeof(operation);
    operation.fMask = SEE_MASK_FLAG_NO_UI;
    operation.hwnd = owner;
    operation.lpVerb = L"open";
    operation.lpFile = L"explorer.exe";
    operation.lpParameters = parameters.c_str();
    operation.nShow = SW_SHOWNORMAL;
    if (::ShellExecuteExW(&operation)) return S_OK;
    return HRESULT_FROM_WIN32(::GetLastError());
  }
  const auto launched = reinterpret_cast<INT_PTR>(
      ::ShellExecuteW(owner, L"open", path_value.c_str(), nullptr, nullptr,
                      SW_SHOWNORMAL));
  if (launched > 32) return S_OK;
  return HRESULT_FROM_WIN32(static_cast<DWORD>(launched));
}

}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project,
                             bool start_hidden)
    : project_(project), start_hidden_(start_hidden) {}

FlutterWindow::~FlutterWindow() {}

int FlutterWindow::RunClipboardSelfTest() {
  // A separate station keeps automated tests away from the user's clipboard.
  const HWINSTA original_station = GetProcessWindowStation();
  const HDESK original_desktop = GetThreadDesktop(GetCurrentThreadId());
  HWINSTA station = CreateWindowStationW(nullptr, 0, WINSTA_ALL_ACCESS, nullptr);
  if (station == nullptr || !SetProcessWindowStation(station)) return 10;
  HDESK desktop = CreateDesktopW(L"NekoSendClipboardTest", nullptr, nullptr, 0, GENERIC_ALL, nullptr);
  if (desktop == nullptr || !SetThreadDesktop(desktop)) {
    SetProcessWindowStation(original_station);
    if (desktop != nullptr) CloseDesktop(desktop);
    CloseWindowStation(station);
    return 11;
  }
  CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  HWND owner = CreateWindowW(L"STATIC", L"", WS_POPUP, 0, 0, 1, 1, nullptr, nullptr, GetModuleHandle(nullptr), nullptr);
  wchar_t temporary[MAX_PATH]{};
  wchar_t old_app_data[32768]{};
  GetTempPathW(MAX_PATH, temporary);
  GetEnvironmentVariableW(L"LOCALAPPDATA", old_app_data, 32768);
  const auto base = std::filesystem::weakly_canonical(temporary);
  const auto root = base / (L"NekoSend-clipboard-test-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64()));
  std::error_code file_error;
  std::filesystem::create_directories(root, file_error);
  SetEnvironmentVariableW(L"LOCALAPPDATA", root.c_str());
  auto set = [owner](UINT format, const void* bytes, size_t count, bool clear) {
    if (!OpenClipboard(owner)) return false;
    if (clear) EmptyClipboard();
    const HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, count);
    void* target = memory == nullptr ? nullptr : GlobalLock(memory);
    if (target == nullptr) { CloseClipboard(); return false; }
    memcpy(target, bytes, count);
    GlobalUnlock(memory);
    const bool success = SetClipboardData(format, memory) != nullptr;
    if (!success) GlobalFree(memory);
    CloseClipboard();
    return success;
  };
  const wchar_t names[] = L"C:\\folder one\\a.txt\0C:\\folder two\\b.png\0\0";
  std::vector<BYTE> drop(sizeof(DROPFILES) + sizeof(names));
  auto* header = reinterpret_cast<DROPFILES*>(drop.data());
  header->pFiles = sizeof(DROPFILES);
  header->fWide = TRUE;
  memcpy(drop.data() + sizeof(DROPFILES), names, sizeof(names));
  bool passed = owner != nullptr && set(CF_HDROP, drop.data(), drop.size(), true);
  const wchar_t text[] = L"C:\\path-only.txt";
  passed = passed && set(CF_UNICODETEXT, text, sizeof(text), false);
  ClipboardReadReply files{nullptr, {}, {}};
  ReadComposerClipboard(owner, &files);
  auto found = files.value.find(flutter::EncodableValue("items"));
  const auto* items = found == files.value.end() ? nullptr : std::get_if<flutter::EncodableList>(&found->second);
  passed = passed && files.error.empty() && items != nullptr && items->size() == 2 &&
      files.value.find(flutter::EncodableValue("text")) == files.value.end();
  passed = passed && set(CF_UNICODETEXT, text, sizeof(text), true);
  ClipboardReadReply plain{nullptr, {}, {}};
  ReadComposerClipboard(owner, &plain);
  passed = passed && plain.error.empty() && plain.value.find(flutter::EncodableValue("text")) != plain.value.end() &&
      plain.value.find(flutter::EncodableValue("items")) == plain.value.end();
  std::vector<BYTE> bitmap(sizeof(BITMAPINFOHEADER) + 16, 0);
  auto* info = reinterpret_cast<BITMAPINFOHEADER*>(bitmap.data());
  info->biSize = sizeof(BITMAPINFOHEADER); info->biWidth = 2; info->biHeight = 2;
  info->biPlanes = 1; info->biBitCount = 32; info->biSizeImage = 16;
  passed = passed && set(CF_DIB, bitmap.data(), bitmap.size(), true);
  ClipboardReadReply image{nullptr, {}, {}};
  ReadComposerClipboard(owner, &image);
  passed = passed && image.error.empty() && image.value.find(flutter::EncodableValue("items")) != image.value.end();
  bool png_found = false;
  if (std::filesystem::exists(root / L"LAN Chat" / L"clipboard-sources")) {
    for (const auto& file : std::filesystem::directory_iterator(root / L"LAN Chat" / L"clipboard-sources")) {
      std::ifstream input(file.path(), std::ios::binary);
      std::array<BYTE, 8> signature{};
      input.read(reinterpret_cast<char*>(signature.data()), 8);
      png_found = signature == std::array<BYTE, 8>{137, 80, 78, 71, 13, 10, 26, 10};
    }
  }
  passed = passed && png_found;
  if (OpenClipboard(owner)) { EmptyClipboard(); CloseClipboard(); }
  DestroyWindow(owner);
  CoUninitialize();
  SetEnvironmentVariableW(L"LOCALAPPDATA", old_app_data);
  SetProcessWindowStation(original_station);
  SetThreadDesktop(original_desktop);
  CloseDesktop(desktop);
  CloseWindowStation(station);
  if (root.parent_path() == base) std::filesystem::remove_all(root, file_error);
  return passed ? 0 : 12;
}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  toast_identity_status_ = EnsureShellIdentity();

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  platform_channel_ = std::make_unique<
      flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "dev.lanchat/platform",
      &flutter::StandardMethodCodec::GetInstance());
  platform_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        if (call.method_name() == "applyAppSettings") {
          const auto* arguments = std::get_if<flutter::EncodableMap>(
              call.arguments());
          const auto read_bool = [arguments](const char* name,
                                             bool fallback) {
            if (arguments == nullptr) return fallback;
            const auto entry =
                arguments->find(flutter::EncodableValue(name));
            if (entry == arguments->end()) return fallback;
            const auto* value = std::get_if<bool>(&entry->second);
            return value == nullptr ? fallback : *value;
          };
          close_to_tray_ = read_bool("closeToTray", true);
          const HRESULT code =
              SetStartAtLogin(read_bool("startOnBoot", false));
          if (SUCCEEDED(code)) {
            result->Success();
          } else {
            result->Error("WINDOWS_STARTUP_SETTING_FAILED",
                          "Windows startup setting could not be updated",
                          flutter::EncodableValue(static_cast<int64_t>(code)));
          }
          return;
        }
        if (call.method_name() == "getDefaultReceiveDirectory") {
          HRESULT code = S_OK;
          auto directory = DefaultReceiveDirectory(&code);
          if (directory.has_value()) {
            result->Success(flutter::EncodableValue(*directory));
          } else {
            result->Error("WINDOWS_RECEIVE_DIRECTORY_FAILED",
                          "Windows receive directory could not be created",
                          flutter::EncodableValue(static_cast<int64_t>(code)));
          }
          return;
        }
        if (call.method_name() == "openReference") {
          const auto* arguments = std::get_if<flutter::EncodableMap>(
              call.arguments());
          const std::string* reference = nullptr;
          bool show_in_folder = false;
          if (arguments != nullptr) {
            const auto reference_entry =
                arguments->find(flutter::EncodableValue("reference"));
            if (reference_entry != arguments->end()) {
              reference = std::get_if<std::string>(&reference_entry->second);
            }
            const auto show_entry =
                arguments->find(flutter::EncodableValue("showInFolder"));
            if (show_entry != arguments->end()) {
              const auto* value = std::get_if<bool>(&show_entry->second);
              show_in_folder = value != nullptr && *value;
            }
          }
          const HRESULT code = reference == nullptr
                                   ? E_INVALIDARG
                                   : OpenLocalReference(
                                         GetHandle(), *reference,
                                         show_in_folder);
          if (SUCCEEDED(code)) {
            result->Success();
          } else {
            result->Error("WINDOWS_OPEN_REFERENCE_FAILED",
                          "Windows could not open the selected item",
                          flutter::EncodableValue(static_cast<int64_t>(code)));
          }
          return;
        }
        if (call.method_name() == "exitApplication") {
          SetQuitOnClose(true);
          Destroy();
          result->Success();
          return;
        }
        if (call.method_name() == "showNotification") {
          const auto* arguments = std::get_if<flutter::EncodableMap>(
              call.arguments());
          const auto read_string = [arguments](const char* name) {
            if (arguments == nullptr) return std::string();
            const auto entry =
                arguments->find(flutter::EncodableValue(name));
            if (entry == arguments->end()) return std::string();
            const auto* value = std::get_if<std::string>(&entry->second);
            return value == nullptr ? std::string() : *value;
          };
          const std::wstring title = Utf16FromUtf8(read_string("title"));
          const std::wstring body = Utf16FromUtf8(read_string("body"));
          notification_route_ = read_string("conversationId");
          if (title.empty()) {
            result->Error("WINDOWS_NOTIFICATION_FAILED",
                          "Windows notification is unavailable");
            return;
          }
          HRESULT toast_code = toast_identity_status_;
          if (SUCCEEDED(toast_code)) {
            toast_code = ShowWindowsToast(GetHandle(), title, body,
                                          notification_route_);
          }
          if (SUCCEEDED(toast_code)) {
            result->Success();
            return;
          }
          if (!tray_icon_added_) {
            result->Error(
                "WINDOWS_NOTIFICATION_FAILED",
                "Windows toast and fallback notification are unavailable",
                flutter::EncodableValue(static_cast<int64_t>(toast_code)));
            return;
          }
          tray_icon_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_INFO;
          wcsncpy_s(tray_icon_.szInfoTitle, title.c_str(), _TRUNCATE);
          wcsncpy_s(tray_icon_.szInfo, body.c_str(), _TRUNCATE);
          tray_icon_.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
          const BOOL shown = ::Shell_NotifyIconW(NIM_MODIFY, &tray_icon_);
          tray_icon_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
          if (shown) {
            result->Success();
          } else {
            result->Error("WINDOWS_NOTIFICATION_FAILED",
                          "Windows notification could not be shown",
                          flutter::EncodableValue(
                              static_cast<int64_t>(::GetLastError())));
          }
          return;
        }
        if (call.method_name() == "readClipboardContent") {
          auto* reply = new ClipboardReadReply{std::move(result), {}, {}};
          const HWND owner = GetHandle();
          std::thread([owner, reply]() {
            const HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
            ReadComposerClipboard(owner, reply);
            if (SUCCEEDED(com)) CoUninitialize();
            if (!PostMessageW(owner, kClipboardReadComplete, 0, reinterpret_cast<LPARAM>(reply))) delete reply;
          }).detach();
          return;
        }
        if (call.method_name() == "readClipboardImage") {
          HRESULT code = S_OK;
          auto source = ReadClipboardImage(GetHandle(), &code);
          if (source.has_value()) {
            flutter::EncodableMap response;
            response[flutter::EncodableValue("displayName")] =
                flutter::EncodableValue(
                    std::filesystem::u8path(source->path)
                        .filename()
                        .u8string());
            response[flutter::EncodableValue("sourceRef")] =
                flutter::EncodableValue(source->path);
            response[flutter::EncodableValue("relativePath")] =
                response[flutter::EncodableValue("displayName")];
            response[flutter::EncodableValue("size")] =
                flutter::EncodableValue(source->size);
            response[flutter::EncodableValue("modifiedAtMs")] =
                flutter::EncodableValue(source->modified_at_ms);
            response[flutter::EncodableValue("fingerprint")] =
                flutter::EncodableValue(source->fingerprint);
            response[flutter::EncodableValue("suppressSync")] =
                flutter::EncodableValue(source->suppress_sync);
            result->Success(flutter::EncodableValue(response));
          } else if (code == S_FALSE) {
            result->Success();
          } else {
            result->Error("READ_CLIPBOARD_IMAGE_FAILED",
                          "Windows clipboard image could not be read",
                          flutter::EncodableValue(static_cast<int64_t>(code)));
          }
          return;
        }
        if (call.method_name() == "writeClipboardImage") {
          const auto* arguments = std::get_if<flutter::EncodableMap>(
              call.arguments());
          const std::string* value = nullptr;
          const std::string* metadata_value = nullptr;
          if (arguments != nullptr) {
            const auto reference =
                arguments->find(flutter::EncodableValue("reference"));
            if (reference != arguments->end()) {
              value = std::get_if<std::string>(&reference->second);
            }
            const auto metadata =
                arguments->find(flutter::EncodableValue("metadata"));
            if (metadata != arguments->end()) {
              metadata_value = std::get_if<std::string>(&metadata->second);
            }
          }
          const HRESULT code =
              value == nullptr || metadata_value == nullptr
                  ? E_INVALIDARG
                  : WriteClipboardImage(
                        GetHandle(), std::filesystem::u8path(*value).wstring(),
                        *metadata_value);
          if (SUCCEEDED(code)) {
            result->Success();
          } else {
            result->Error("WRITE_CLIPBOARD_IMAGE_FAILED",
                          "Windows clipboard image could not be written",
                          flutter::EncodableValue(static_cast<int64_t>(code)));
          }
          return;
        }
        if (call.method_name() != "pickSource") {
          result->NotImplemented();
          return;
        }
        bool folder = false;
        const auto* arguments = std::get_if<flutter::EncodableMap>(
            call.arguments());
        if (arguments != nullptr) {
          const auto kind = arguments->find(flutter::EncodableValue("kind"));
          if (kind != arguments->end()) {
            const auto* value = std::get_if<std::string>(&kind->second);
            folder = value != nullptr && *value == "folder";
          }
        }
        HRESULT code = S_OK;
        auto selected = PickSource(GetHandle(), folder, &code);
        if (selected.has_value()) {
          result->Success(flutter::EncodableValue(*selected));
        } else if (code == HRESULT_FROM_WIN32(ERROR_CANCELLED)) {
          result->Success();
        } else {
          result->Error("PICK_SOURCE_FAILED", "Windows file dialog failed",
                        flutter::EncodableValue(static_cast<int64_t>(code)));
        }
      });
  ::AddClipboardFormatListener(GetHandle());
  if (::NotifyIpInterfaceChange(AF_INET, NetworkChangedCallback, GetHandle(),
                                FALSE,
                                &network_change_handle_) != NO_ERROR) {
    network_change_handle_ = nullptr;
  }
  tray_icon_.cbSize = sizeof(tray_icon_);
  tray_icon_.hWnd = GetHandle();
  tray_icon_.uID = 1;
  tray_icon_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
  tray_icon_.uCallbackMessage = kTrayCallbackMessage;
  tray_icon_.hIcon = static_cast<HICON>(::LoadImageW(
      ::GetModuleHandle(nullptr), MAKEINTRESOURCEW(IDI_APP_ICON), IMAGE_ICON,
      16, 16, LR_DEFAULTCOLOR));
  wcscpy_s(tray_icon_.szTip, L"\x732B\x732B\x5FEB\x4F20");
  tray_icon_added_ = ::Shell_NotifyIconW(NIM_ADD, &tray_icon_) == TRUE;
  if (tray_icon_added_) {
    tray_icon_.uVersion = NOTIFYICON_VERSION_4;
    ::Shell_NotifyIconW(NIM_SETVERSION, &tray_icon_);
  }
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    if (!start_hidden_) {
      this->Show();
    }
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OnDestroy() {
  if (network_change_handle_ != nullptr) {
    ::CancelMibChangeNotify2(network_change_handle_);
    network_change_handle_ = nullptr;
  }
  ::RemoveClipboardFormatListener(GetHandle());
  if (tray_icon_added_) {
    ::Shell_NotifyIconW(NIM_DELETE, &tray_icon_);
    tray_icon_added_ = false;
  }
  platform_channel_.reset();
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

void FlutterWindow::OpenConversationRoute(const std::string& route) {
  ShowMainWindow();
  constexpr char kOpenConversationPrefix[] = "openConversation:";
  if (platform_channel_ && route.rfind(kOpenConversationPrefix, 0) == 0) {
    platform_channel_->InvokeMethod(
        "notificationOpenConversation",
        std::make_unique<flutter::EncodableValue>(
            route.substr(sizeof(kOpenConversationPrefix) - 1)));
  }
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case kClipboardReadComplete: {
      std::unique_ptr<ClipboardReadReply> reply(reinterpret_cast<ClipboardReadReply*>(lparam));
      if (reply->error.empty()) reply->result->Success(flutter::EncodableValue(reply->value));
      else reply->result->Error("CLIPBOARD_READ_FAILED", reply->error);
      return 0;
    }
    case kNetworkChangedMessage:
      InvokeDartAction("networkChanged");
      return 0;
    case kToastActivatedMessage: {
      std::unique_ptr<std::string> route(
          reinterpret_cast<std::string*>(lparam));
      if (route) OpenConversationRoute(*route);
      return 0;
    }
    case WM_COPYDATA: {
      const auto* data = reinterpret_cast<const COPYDATASTRUCT*>(lparam);
      if (data == nullptr || data->dwData != kLanChatOpenConversationCopyData ||
          data->lpData == nullptr || data->cbData < 2 || data->cbData > 4096) {
        return FALSE;
      }
      const auto* bytes = static_cast<const char*>(data->lpData);
      if (bytes[data->cbData - 1] != '\0') return FALSE;
      OpenConversationRoute(std::string(bytes));
      return TRUE;
    }
    case WM_POWERBROADCAST:
      if (wparam == PBT_APMRESUMEAUTOMATIC ||
          wparam == PBT_APMRESUMESUSPEND) {
        InvokeDartAction("networkChanged");
      }
      return TRUE;
    case WM_CLOSE:
      if (close_to_tray_) {
        ::ShowWindow(GetHandle(), SW_HIDE);
      } else {
        InvokeDartAction("windowExitRequested");
      }
      return 0;
    case WM_COMMAND:
      switch (LOWORD(wparam)) {
        case kTrayOpen:
          ShowMainWindow();
          return 0;
        case kTraySendClipboard:
          InvokeDartAction("traySendClipboard");
          return 0;
        case kTrayPauseAll:
          InvokeDartAction("trayPauseAllTransfers");
          return 0;
        case kTrayExit:
          ShowMainWindow();
          InvokeDartAction("trayExitRequested");
          return 0;
      }
      break;
    case kTrayCallbackMessage:
      switch (LOWORD(lparam)) {
        case WM_LBUTTONDBLCLK:
          ShowMainWindow();
          return 0;
        case WM_CONTEXTMENU:
          ShowTrayMenu();
          return 0;
        case NIN_BALLOONUSERCLICK:
          ShowMainWindow();
          if (platform_channel_ && !notification_route_.empty()) {
            platform_channel_->InvokeMethod(
                "notificationOpenConversation",
                std::make_unique<flutter::EncodableValue>(
                    notification_route_));
          }
          return 0;
      }
      break;
    case WM_CLIPBOARDUPDATE:
      if (platform_channel_) {
        platform_channel_->InvokeMethod("clipboardChanged", nullptr);
      }
      break;
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}

void FlutterWindow::ShowMainWindow() {
  ::ShowWindow(GetHandle(), SW_RESTORE);
  ::SetForegroundWindow(GetHandle());
}

void FlutterWindow::ShowTrayMenu() {
  HMENU menu = ::CreatePopupMenu();
  if (menu == nullptr) {
    return;
  }
  ::AppendMenuW(menu, MF_STRING, kTrayOpen,
                L"\x6253\x5F00\x732B\x732B\x5FEB\x4F20");
  ::AppendMenuW(menu, MF_STRING, kTraySendClipboard,
                L"\x53D1\x9001\x526A\x8D34\x677F");
  ::AppendMenuW(menu, MF_STRING, kTrayPauseAll,
                L"\x6682\x505C\x5168\x90E8\x4F20\x8F93");
  ::AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  ::AppendMenuW(menu, MF_STRING, kTrayExit, L"\x9000\x51FA");
  POINT cursor{};
  ::GetCursorPos(&cursor);
  ::SetForegroundWindow(GetHandle());
  ::TrackPopupMenu(menu, TPM_RIGHTBUTTON, cursor.x, cursor.y, 0, GetHandle(),
                   nullptr);
  ::DestroyMenu(menu);
}

void FlutterWindow::InvokeDartAction(const std::string& method) {
  if (platform_channel_) {
    platform_channel_->InvokeMethod(method, nullptr);
  }
}
