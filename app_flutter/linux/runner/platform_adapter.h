#pragma once

#include <flutter_linux/flutter_linux.h>
#include <gtk/gtk.h>

struct NekoPlatform;

NekoPlatform* neko_platform_new(GtkApplication* application, GtkWindow* window, FlEngine* engine);
void neko_platform_free(NekoPlatform* platform);
void neko_platform_ready(NekoPlatform* platform);
void neko_platform_open_conversation(NekoPlatform* platform, const char* id);
void neko_platform_request_exit(NekoPlatform* platform);
gboolean neko_platform_window_close(NekoPlatform* platform);
