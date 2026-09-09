#ifndef RUNNER_SHELL_IDENTITY_H_
#define RUNNER_SHELL_IDENTITY_H_

#include <windows.h>

#ifdef _DEBUG
inline constexpr wchar_t kAppUserModelId[] = L"dev.lanchat.LANChat.Debug";
inline constexpr wchar_t kAppShortcutName[] = L"NekoSend Debug.lnk";
#else
inline constexpr wchar_t kAppUserModelId[] = L"dev.lanchat.LANChat";
inline constexpr wchar_t kAppShortcutName[] = L"NekoSend.lnk";
#endif

// Creates the canonical shortcut and migrates only application-owned legacy links.
HRESULT EnsureShellIdentity(const wchar_t* executable_override = nullptr,
                            const wchar_t* programs_override = nullptr);

#endif  // RUNNER_SHELL_IDENTITY_H_
