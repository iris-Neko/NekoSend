#include "platform_adapter.h"

#include <glib/gstdio.h>
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <string>
#include <map>
#include <vector>
#include <thread>

namespace {
constexpr gsize kMaxClipboardBytes = 20 * 1024 * 1024;
constexpr int64_t kMaxClipboardPixels = 32 * 1024 * 1024;
constexpr char kOriginTarget[] = "application/x-nekosend-clipboard-origin";
constexpr char kPlatformKey[] = "nekosend-platform-adapter";

std::string Join(const char* parent, const char* child) {
  g_autofree gchar* path = g_build_filename(parent, child, nullptr);
  return path;
}

std::string StringArg(FlValue* args, const char* key) {
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) return {};
  FlValue* value = fl_value_lookup_string(args, key);
  return value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_STRING
             ? fl_value_get_string(value) : "";
}

bool BoolArg(FlValue* args, const char* key, bool fallback = false) {
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) return fallback;
  FlValue* value = fl_value_lookup_string(args, key);
  return value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_BOOL
             ? fl_value_get_bool(value) : fallback;
}

void Success(FlMethodCall* call, FlValue* value = nullptr) {
  g_autoptr(FlMethodResponse) response = FL_METHOD_RESPONSE(fl_method_success_response_new(value));
  fl_method_call_respond(call, response, nullptr);
}

void Failure(FlMethodCall* call, const char* code, const char* message) {
  g_autoptr(FlMethodResponse) response = FL_METHOD_RESPONSE(fl_method_error_response_new(code, message, nullptr));
  fl_method_call_respond(call, response, nullptr);
}

struct ClipboardContent {
  GdkPixbuf* image;
  std::string metadata;
};

void ClipboardData(GtkClipboard*, GtkSelectionData* selection, guint info, gpointer data) {
  auto* content = static_cast<ClipboardContent*>(data);
  if (info == 1) {
    gtk_selection_data_set_pixbuf(selection, content->image);
  } else {
    gtk_selection_data_set(selection, gdk_atom_intern_static_string(kOriginTarget), 8,
        reinterpret_cast<const guchar*>(content->metadata.data()), static_cast<gint>(content->metadata.size()));
  }
}

void ClearClipboard(GtkClipboard*, gpointer data) {
  auto* content = static_cast<ClipboardContent*>(data);
  g_object_unref(content->image);
  delete content;
}

GVariant* MenuNode(gint id, const char* label, bool enabled, const char* icon) {
  GVariantBuilder properties;
  g_variant_builder_init(&properties, G_VARIANT_TYPE("a{sv}"));
  g_variant_builder_add(&properties, "{sv}", "label", g_variant_new_string(label));
  g_variant_builder_add(&properties, "{sv}", "enabled", g_variant_new_boolean(enabled));
  g_variant_builder_add(&properties, "{sv}", "visible", g_variant_new_boolean(TRUE));
  if (icon != nullptr) g_variant_builder_add(&properties, "{sv}", "icon-name", g_variant_new_string(icon));
  GVariantBuilder children;
  g_variant_builder_init(&children, G_VARIANT_TYPE("av"));
  return g_variant_new("(i@a{sv}@av)", id, g_variant_builder_end(&properties), g_variant_builder_end(&children));
}

constexpr char kTrayXml[] = R"XML(
<node><interface name="org.kde.StatusNotifierItem">
 <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
 <property name="Category" type="s" access="read"/><property name="Id" type="s" access="read"/>
 <property name="Title" type="s" access="read"/><property name="Status" type="s" access="read"/>
 <property name="WindowId" type="u" access="read"/><property name="IconName" type="s" access="read"/>
 <property name="IconPixmap" type="a(iiay)" access="read"/>
 <property name="AttentionIconName" type="s" access="read"/><property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
 <property name="OverlayIconName" type="s" access="read"/><property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
 <property name="ToolTip" type="(sa(iiay)ss)" access="read"/><property name="ItemIsMenu" type="b" access="read"/>
 <property name="Menu" type="o" access="read"/>
 <signal name="NewIcon"/><signal name="NewToolTip"/><signal name="NewStatus"><arg type="s"/></signal>
</interface></node>)XML";

constexpr char kMenuXml[] = R"XML(
<node><interface name="com.canonical.dbusmenu">
 <method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
 <method name="GetGroupProperties"><arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/></method>
 <method name="GetProperty"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
 <method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
 <method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
 <method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
 <method name="AboutToShowGroup"><arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/></method>
 <property name="Version" type="u" access="read"/><property name="TextDirection" type="s" access="read"/>
 <property name="Status" type="s" access="read"/><property name="IconThemePath" type="as" access="read"/>
 <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
</interface></node>)XML";
}  // namespace

struct NekoPlatform {
  GtkApplication* application;
  GtkWindow* window;
  FlMethodChannel* channel = nullptr;
  GtkClipboard* clipboard;
  GNetworkMonitor* network;
  GDBusConnection* bus = nullptr;
  GCancellable* cancellable = g_cancellable_new();
  GtkFileChooserNative* picker = nullptr;
  FlMethodCall* picker_call = nullptr;
  FlMethodCall* autostart_call = nullptr;
  GDBusNodeInfo* tray_info = nullptr;
  GDBusNodeInfo* menu_info = nullptr;
  guint tray_registration = 0;
  guint menu_registration = 0;
  guint watcher_subscription = 0;
  guint portal_subscription = 0;
  gulong clipboard_signal = 0;
  gulong network_signal = 0;
  guint menu_revision = 1;
  bool ready = false;
  bool close_to_tray = true;
  bool notifications = true;
  bool settings_seen = false;
  bool start_on_boot = false;
  bool requested_autostart = false;
  int active_transfers = 0;
  std::string data_directory;
  std::string portal_path;
  std::string pending_conversation;
  guint clipboard_revision = 0;
  std::map<std::string, std::vector<std::string>> composer_portal_cache;

  struct ComposerRead {
    GtkApplication* app;
    GtkClipboard* clipboard;
    FlMethodCall* call;
    guint revision;
    std::string directory;
    std::string portal_key;
    std::string path;
    std::string hash;
    std::string error;
    ~ComposerRead() { g_object_unref(app); g_object_unref(clipboard); g_object_unref(call); }
  };

  static NekoPlatform* CurrentComposer(ComposerRead* read) {
    auto* self = static_cast<NekoPlatform*>(g_object_get_data(G_OBJECT(read->app), kPlatformKey));
    if (self == nullptr || self->clipboard_revision != read->revision) {
      Failure(read->call, "CLIPBOARD_CHANGED", "Clipboard changed; paste again");
      delete read;
      return nullptr;
    }
    return self;
  }

  static void ComposerFiles(ComposerRead* read, const std::vector<std::string>& paths, bool ephemeral) {
    g_autoptr(FlValue) response = fl_value_new_map();
    g_autoptr(FlValue) items = fl_value_new_list();
    size_t bytes = 0;
    for (const auto& path : paths) bytes += path.size();
    if (paths.size() > 10000 || bytes > 16 * 1024 * 1024) {
      Failure(read->call, "CLIPBOARD_TOO_LARGE", "Clipboard file list is too large"); delete read; return;
    }
    for (const auto& path : paths) {
      if (!g_utf8_validate(path.c_str(), static_cast<gssize>(path.size()), nullptr)) {
        Failure(read->call, "CLIPBOARD_INVALID_PATH", "File path is not valid UTF-8"); delete read; return;
      }
      g_autofree gchar* name = g_path_get_basename(path.c_str());
      FlValue* item = fl_value_new_map();
      fl_value_set_string_take(item, "sourceRef", fl_value_new_string(path.c_str()));
      fl_value_set_string_take(item, "displayName", fl_value_new_string(name));
      fl_value_set_string_take(item, "ephemeral", fl_value_new_bool(ephemeral));
      fl_value_append_take(items, item);
    }
    fl_value_set_string(response, "items", items);
    Success(read->call, response);
    delete read;
  }

  static void ComposerText(GtkClipboard*, const gchar* text, gpointer data) {
    auto* read = static_cast<ComposerRead*>(data);
    if (CurrentComposer(read) == nullptr) return;
    g_autoptr(FlValue) response = fl_value_new_map();
    fl_value_set_string_take(response, "text", fl_value_new_string(text == nullptr ? "" : text));
    Success(read->call, response);
    delete read;
  }

  static void ComposerImage(GtkClipboard*, GdkPixbuf* image, gpointer data) {
    auto* read = static_cast<ComposerRead*>(data);
    if (CurrentComposer(read) == nullptr) return;
    if (image == nullptr) { gtk_clipboard_request_text(read->clipboard, ComposerText, read); return; }
    const int64_t pixels = static_cast<int64_t>(gdk_pixbuf_get_width(image)) * gdk_pixbuf_get_height(image);
    if (pixels <= 0 || pixels > kMaxClipboardPixels) {
      Failure(read->call, "CLIPBOARD_TOO_LARGE", "Clipboard image dimensions are too large"); delete read; return;
    }
    g_object_ref(image);
    std::thread([read, image]() {
      gchar* bytes = nullptr;
      gsize length = 0;
      GError* error = nullptr;
      if (!gdk_pixbuf_save_to_buffer(image, &bytes, &length, "png", &error, nullptr)) {
        read->error = error == nullptr ? "Cannot encode clipboard image" : error->message;
      } else if (length > kMaxClipboardBytes) {
        read->error = "Clipboard image exceeds 20 MiB";
      } else {
        gchar* hash = g_compute_checksum_for_data(G_CHECKSUM_SHA256, reinterpret_cast<const guchar*>(bytes), length);
        read->hash = hash;
        g_free(hash);
        const auto directory = Join(read->directory.c_str(), "clipboard");
        g_mkdir_with_parents(directory.c_str(), 0700);
        read->path = Join(directory.c_str(), (read->hash + ".png").c_str());
        if (!g_file_test(read->path.c_str(), G_FILE_TEST_EXISTS) &&
            !g_file_set_contents(read->path.c_str(), bytes, static_cast<gssize>(length), &error)) read->error = error->message;
      }
      g_clear_error(&error);
      g_free(bytes);
      g_object_unref(image);
      g_main_context_invoke(nullptr, [](gpointer raw) -> gboolean {
        auto* completed = static_cast<ComposerRead*>(raw);
        if (CurrentComposer(completed) == nullptr) return G_SOURCE_REMOVE;
        if (!completed->error.empty()) {
          Failure(completed->call, "CLIPBOARD_READ_FAILED", completed->error.c_str());
        } else {
          g_autoptr(FlValue) response = fl_value_new_map();
          g_autoptr(FlValue) items = fl_value_new_list();
          FlValue* item = fl_value_new_map();
          fl_value_set_string_take(item, "sourceRef", fl_value_new_string(completed->path.c_str()));
          fl_value_set_string_take(item, "displayName", fl_value_new_string("clipboard.png"));
          fl_value_set_string_take(item, "kind", fl_value_new_string("image"));
          fl_value_set_string_take(item, "ephemeral", fl_value_new_bool(true));
          fl_value_set_string_take(item, "fingerprint", fl_value_new_string(completed->hash.c_str()));
          fl_value_append_take(items, item);
          fl_value_set_string(response, "items", items);
          Success(completed->call, response);
        }
        delete completed;
        return G_SOURCE_REMOVE;
      }, read);
    }).detach();
  }

  static void ComposerUris(GtkClipboard*, gchar** uris, gpointer data) {
    auto* read = static_cast<ComposerRead*>(data);
    if (CurrentComposer(read) == nullptr) return;
    std::vector<std::string> paths;
    if (uris != nullptr) for (gchar** uri = uris; *uri != nullptr; ++uri) {
      g_autofree gchar* path = g_filename_from_uri(*uri, nullptr, nullptr);
      if (path != nullptr) paths.emplace_back(path);
    }
    if (!paths.empty()) ComposerFiles(read, paths, false);
    else gtk_clipboard_request_image(read->clipboard, ComposerImage, read);
  }

  static void ComposerPortal(GtkClipboard*, GtkSelectionData* selection, gpointer data) {
    auto* read = static_cast<ComposerRead*>(data);
    auto* self = CurrentComposer(read);
    if (self == nullptr) return;
    const int length = gtk_selection_data_get_length(selection);
    if (length <= 0) { gtk_clipboard_request_uris(read->clipboard, ComposerUris, read); return; }
    if (length > 4096 || self->bus == nullptr) {
      Failure(read->call, "CLIPBOARD_PORTAL_FAILED", "File portal is unavailable"); delete read; return;
    }
    read->portal_key.assign(reinterpret_cast<const char*>(gtk_selection_data_get_data(selection)), static_cast<size_t>(length));
    while (!read->portal_key.empty() && read->portal_key.back() == '\0') read->portal_key.pop_back();
    if (read->portal_key.empty() || read->portal_key.find('\0') != std::string::npos ||
        !g_utf8_validate(read->portal_key.c_str(), static_cast<gssize>(read->portal_key.size()), nullptr)) {
      Failure(read->call, "CLIPBOARD_PORTAL_FAILED", "Invalid file portal token"); delete read; return;
    }
    const auto cached = self->composer_portal_cache.find(read->portal_key);
    if (cached != self->composer_portal_cache.end()) { ComposerFiles(read, cached->second, true); return; }
    g_dbus_connection_call(self->bus, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
      "org.freedesktop.portal.FileTransfer", "RetrieveFiles",
      g_variant_new("(s@a{sv})", read->portal_key.c_str(), g_variant_new_array(G_VARIANT_TYPE("{sv}"), nullptr, 0)),
      G_VARIANT_TYPE("(as)"), G_DBUS_CALL_FLAGS_NONE, 10000, self->cancellable,
      [](GObject* object, GAsyncResult* result, gpointer raw) {
        auto* pending = static_cast<ComposerRead*>(raw);
        g_autoptr(GError) error = nullptr;
        g_autoptr(GVariant) reply = g_dbus_connection_call_finish(G_DBUS_CONNECTION(object), result, &error);
        auto* owner = CurrentComposer(pending);
        if (owner == nullptr) return;
        if (reply == nullptr) {
          Failure(pending->call, "CLIPBOARD_PORTAL_FAILED", error->message); delete pending; return;
        }
        gchar** files = nullptr;
        g_variant_get(reply, "(^as)", &files);
        std::vector<std::string> paths;
        for (gchar** file = files; file != nullptr && *file != nullptr; ++file) paths.emplace_back(*file);
        g_strfreev(files);
        owner->composer_portal_cache[pending->portal_key] = paths;
        ComposerFiles(pending, paths, true);
      }, read);
  }

  void ReadComposer(FlMethodCall* call) {
    auto* read = new ComposerRead{GTK_APPLICATION(g_object_ref(application)),
      GTK_CLIPBOARD(g_object_ref(clipboard)), FL_METHOD_CALL(g_object_ref(call)), clipboard_revision,
      data_directory, {}, {}, {}, {}};
    gtk_clipboard_request_contents(clipboard, gdk_atom_intern_static_string("application/vnd.portal.filetransfer"), ComposerPortal, read);
  }

  NekoPlatform(GtkApplication* app, GtkWindow* app_window, FlEngine* engine)
      : application(app), window(app_window),
        clipboard(gtk_clipboard_get(GDK_SELECTION_CLIPBOARD)),
        network(G_NETWORK_MONITOR(g_object_ref(g_network_monitor_get_default()))),
        data_directory(Join(g_get_user_data_dir(), "NekoSend")) {
    g_mkdir_with_parents(data_directory.c_str(), 0700);
    g_object_set_data(G_OBJECT(application), kPlatformKey, this);
    g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
    channel = fl_method_channel_new(fl_engine_get_binary_messenger(engine), "dev.lanchat/platform", FL_METHOD_CODEC(codec));
    fl_method_channel_set_method_call_handler(channel, MethodCall, this, nullptr);
    clipboard_signal = g_signal_connect(clipboard, "owner-change", G_CALLBACK(ClipboardChanged), this);
    network_signal = g_signal_connect(network, "network-changed", G_CALLBACK(NetworkChanged), this);
    GDBusConnection* connection = g_application_get_dbus_connection(G_APPLICATION(app));
    if (connection != nullptr) {
      bus = G_DBUS_CONNECTION(g_object_ref(connection));
      SetupTray();
    }
  }

  ~NekoPlatform() {
    g_object_set_data(G_OBJECT(application), kPlatformKey, nullptr);
    g_cancellable_cancel(cancellable);
    if (picker != nullptr) {
      g_signal_handlers_disconnect_by_data(picker, this);
      gtk_native_dialog_destroy(GTK_NATIVE_DIALOG(picker));
      g_clear_object(&picker);
    }
    g_clear_object(&picker_call);
    g_clear_object(&autostart_call);
    if (bus != nullptr) {
      if (portal_subscription) g_dbus_connection_signal_unsubscribe(bus, portal_subscription);
      if (watcher_subscription) g_dbus_connection_signal_unsubscribe(bus, watcher_subscription);
      if (tray_registration) g_dbus_connection_unregister_object(bus, tray_registration);
      if (menu_registration) g_dbus_connection_unregister_object(bus, menu_registration);
    }
    if (clipboard_signal) g_signal_handler_disconnect(clipboard, clipboard_signal);
    if (network_signal) g_signal_handler_disconnect(network, network_signal);
    gtk_clipboard_store(clipboard);
    fl_method_channel_set_method_call_handler(channel, nullptr, nullptr, nullptr);
    g_clear_object(&channel);
    g_clear_object(&network);
    g_clear_object(&bus);
    g_clear_object(&cancellable);
    g_clear_pointer(&tray_info, g_dbus_node_info_unref);
    g_clear_pointer(&menu_info, g_dbus_node_info_unref);
  }

  void Emit(const char* method, FlValue* args = nullptr) {
    if (ready) fl_method_channel_invoke_method(channel, method, args, nullptr, nullptr, nullptr);
  }

  void Present() {
    gtk_widget_show(GTK_WIDGET(window));
    gtk_window_present(window);
  }

  void RequestExit() {
    if (!ready) { g_application_quit(G_APPLICATION(application)); return; }
    Present();
    Emit("trayExitRequested");
  }

  static void MethodCall(FlMethodChannel*, FlMethodCall* call, gpointer data) {
    static_cast<NekoPlatform*>(data)->Handle(call);
  }

  void Handle(FlMethodCall* call) {
    const std::string method = fl_method_call_get_name(call);
    FlValue* args = fl_method_call_get_args(call);
    if (method == "getBootstrapInfo") {
      g_autoptr(FlValue) result = fl_value_new_map();
      fl_value_set_string_take(result, "dataDirectory", fl_value_new_string(data_directory.c_str()));
      fl_value_set_string_take(result, "deviceName", fl_value_new_string(g_get_host_name()));
      Success(call, result);
    } else if (method == "getDefaultReceiveDirectory") {
      const char* downloads = g_get_user_special_dir(G_USER_DIRECTORY_DOWNLOAD);
      std::string parent = downloads != nullptr ? downloads : Join(g_get_home_dir(), "Downloads");
      const std::string directory = Join(parent.c_str(), "NekoSend");
      if (g_mkdir_with_parents(directory.c_str(), 0700) != 0) {
        Success(call);
      } else {
        g_autoptr(FlValue) result = fl_value_new_string(directory.c_str());
        Success(call, result);
      }
    } else if (method == "pickSource") {
      PickSource(call, StringArg(args, "kind"));
    } else if (method == "openReference") {
      OpenReference(call, StringArg(args, "reference"), BoolArg(args, "showInFolder"));
    } else if (method == "readClipboardContent") {
      ReadComposer(call);
    } else if (method == "readClipboardImage") {
      ReadClipboardImage(call);
    } else if (method == "writeClipboardImage") {
      WriteClipboardImage(call, StringArg(args, "reference"), StringArg(args, "metadata"));
    } else if (method == "applyAppSettings") {
      close_to_tray = BoolArg(args, "closeToTray", true);
      notifications = BoolArg(args, "notificationsEnabled", true);
      const bool next_autostart = BoolArg(args, "startOnBoot");
      if (!settings_seen || next_autostart == start_on_boot) {
        settings_seen = true;
        start_on_boot = next_autostart;
        Success(call);
      } else {
        RequestAutostart(call, next_autostart);
      }
    } else if (method == "updateActiveTransferCount") {
      FlValue* count = args == nullptr ? nullptr : fl_value_lookup_string(args, "count");
      active_transfers = count != nullptr && fl_value_get_type(count) == FL_VALUE_TYPE_INT
                             ? static_cast<int>(fl_value_get_int(count)) : 0;
      if (bus != nullptr && menu_registration) {
        g_dbus_connection_emit_signal(bus, nullptr, "/MenuBar", "com.canonical.dbusmenu", "LayoutUpdated",
                                     g_variant_new("(ui)", ++menu_revision, 0), nullptr);
      }
      Success(call);
    } else if (method == "showNotification") {
      ShowNotification(args);
      Success(call);
    } else if (method == "exitApplication") {
      Success(call);
      g_idle_add([](gpointer app) -> gboolean {
        g_application_quit(G_APPLICATION(app));
        return G_SOURCE_REMOVE;
      }, application);
    } else {
      g_autoptr(FlMethodResponse) response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
      fl_method_call_respond(call, response, nullptr);
    }
  }

  void PickSource(FlMethodCall* call, const std::string& kind) {
    if (picker != nullptr) { Failure(call, "PICKER_BUSY", "A file chooser is already open"); return; }
    if (kind != "file" && kind != "image" && kind != "folder") {
      Failure(call, "INVALID_ARGUMENT", "Unknown source kind"); return;
    }
    picker = gtk_file_chooser_native_new(kind == "folder" ? "Select folder" : "Select file", window,
        kind == "folder" ? GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER : GTK_FILE_CHOOSER_ACTION_OPEN,
        nullptr, nullptr);
    if (kind == "image") {
      GtkFileFilter* filter = gtk_file_filter_new();
      gtk_file_filter_set_name(filter, "Images");
      gtk_file_filter_add_pixbuf_formats(filter);
      gtk_file_chooser_add_filter(GTK_FILE_CHOOSER(picker), filter);
    }
    picker_call = FL_METHOD_CALL(g_object_ref(call));
    g_signal_connect(picker, "response", G_CALLBACK(PickerResponse), this);
    gtk_native_dialog_show(GTK_NATIVE_DIALOG(picker));
  }

  static void PickerResponse(GtkNativeDialog*, gint response, gpointer data) {
    auto* self = static_cast<NekoPlatform*>(data);
    if (response == GTK_RESPONSE_ACCEPT) {
      g_autoptr(GFile) file = gtk_file_chooser_get_file(GTK_FILE_CHOOSER(self->picker));
      g_autofree gchar* path = file == nullptr ? nullptr : g_file_get_path(file);
      if (path == nullptr) {
        Failure(self->picker_call, "SOURCE_UNAVAILABLE", "The selected item has no accessible local path");
      } else {
        g_autoptr(FlValue) result = fl_value_new_string(path);
        Success(self->picker_call, result);
      }
    } else {
      Success(self->picker_call);
    }
    g_signal_handlers_disconnect_by_data(self->picker, self);
    gtk_native_dialog_destroy(GTK_NATIVE_DIALOG(self->picker));
    g_clear_object(&self->picker);
    g_clear_object(&self->picker_call);
  }

  void OpenReference(FlMethodCall* call, const std::string& reference, bool in_folder) {
    if (reference.empty() || !g_path_is_absolute(reference.c_str()) ||
        !g_file_test(reference.c_str(), G_FILE_TEST_EXISTS)) {
      Failure(call, "FILE_UNAVAILABLE", "The local file or folder does not exist"); return;
    }
    g_autoptr(GFile) file = g_file_new_for_path(reference.c_str());
    if (in_folder && !g_file_test(reference.c_str(), G_FILE_TEST_IS_DIR)) {
      GFile* parent = g_file_get_parent(file);
      g_clear_object(&file);
      file = parent;
    }
    g_autofree gchar* uri = g_file_get_uri(file);
    g_autoptr(GError) error = nullptr;
    if (!gtk_show_uri_on_window(window, uri, GDK_CURRENT_TIME, &error)) {
      Failure(call, "OPEN_FAILED", error->message);
    } else {
      Success(call);
    }
  }

  void ReadClipboardImage(FlMethodCall* call) {
    g_autoptr(GdkPixbuf) image = gtk_clipboard_wait_for_image(clipboard);
    if (image == nullptr) { Success(call); return; }
    const int64_t pixels = static_cast<int64_t>(gdk_pixbuf_get_width(image)) * gdk_pixbuf_get_height(image);
    if (pixels <= 0 || pixels > kMaxClipboardPixels) {
      Failure(call, "CLIPBOARD_TOO_LARGE", "Clipboard image dimensions are too large"); return;
    }
    g_autofree gchar* bytes = nullptr;
    gsize length = 0;
    g_autoptr(GError) error = nullptr;
    if (!gdk_pixbuf_save_to_buffer(image, &bytes, &length, "png", &error, nullptr)) {
      Failure(call, "CLIPBOARD_READ_FAILED", error->message); return;
    }
    if (length > kMaxClipboardBytes) {
      Failure(call, "CLIPBOARD_TOO_LARGE", "Clipboard images must not exceed 20 MiB"); return;
    }
    g_autofree gchar* fingerprint = g_compute_checksum_for_data(G_CHECKSUM_SHA256,
        reinterpret_cast<const guchar*>(bytes), length);
    const std::string directory = Join(data_directory.c_str(), "clipboard");
    g_mkdir_with_parents(directory.c_str(), 0700);
    const std::string name = std::string("clipboard-") + fingerprint + ".png";
    const std::string path = Join(directory.c_str(), name.c_str());
    if (!g_file_test(path.c_str(), G_FILE_TEST_EXISTS) &&
        !g_file_set_contents(path.c_str(), bytes, static_cast<gssize>(length), &error)) {
      Failure(call, "CLIPBOARD_CACHE_FAILED", error->message); return;
    }
    GStatBuf file_stat;
    if (g_stat(path.c_str(), &file_stat) != 0) {
      Failure(call, "CLIPBOARD_CACHE_FAILED", "Unable to inspect cached clipboard image"); return;
    }
    GtkSelectionData* origin = gtk_clipboard_wait_for_contents(clipboard, gdk_atom_intern_static_string(kOriginTarget));
    const bool suppressed = origin != nullptr && gtk_selection_data_get_length(origin) > 0;
    if (origin != nullptr) gtk_selection_data_free(origin);
    g_autoptr(FlValue) result = fl_value_new_map();
    fl_value_set_string_take(result, "displayName", fl_value_new_string("clipboard.png"));
    fl_value_set_string_take(result, "relativePath", fl_value_new_string("clipboard.png"));
    fl_value_set_string_take(result, "sourceRef", fl_value_new_string(path.c_str()));
    fl_value_set_string_take(result, "fingerprint", fl_value_new_string(fingerprint));
    fl_value_set_string_take(result, "size", fl_value_new_int(static_cast<int64_t>(length)));
    const int64_t modified_ms = static_cast<int64_t>(file_stat.st_mtim.tv_sec) * 1000 + file_stat.st_mtim.tv_nsec / 1000000;
    fl_value_set_string_take(result, "modifiedAtMs", fl_value_new_int(modified_ms));
    fl_value_set_string_take(result, "suppressSync", fl_value_new_bool(suppressed));
    Success(call, result);
  }

  void WriteClipboardImage(FlMethodCall* call, const std::string& reference, const std::string& metadata) {
    GStatBuf file_stat;
    int width = 0;
    int height = 0;
    if (!g_path_is_absolute(reference.c_str()) || g_stat(reference.c_str(), &file_stat) != 0 ||
        file_stat.st_size <= 0 || static_cast<uint64_t>(file_stat.st_size) > kMaxClipboardBytes ||
        gdk_pixbuf_get_file_info(reference.c_str(), &width, &height) == nullptr ||
        width <= 0 || height <= 0 || static_cast<int64_t>(width) * height > kMaxClipboardPixels) {
      Failure(call, "CLIPBOARD_IMAGE_INVALID", "Clipboard image is unavailable or too large"); return;
    }
    g_autoptr(GError) error = nullptr;
    GdkPixbuf* image = gdk_pixbuf_new_from_file(reference.c_str(), &error);
    if (image == nullptr) { Failure(call, "CLIPBOARD_WRITE_FAILED", error->message); return; }
    auto* content = new ClipboardContent{image, metadata};
    GtkTargetList* list = gtk_target_list_new(nullptr, 0);
    gtk_target_list_add_image_targets(list, 1, TRUE);
    gtk_target_list_add(list, gdk_atom_intern_static_string(kOriginTarget), 0, 2);
    gint count = 0;
    GtkTargetEntry* targets = gtk_target_table_new_from_list(list, &count);
    const gboolean set = gtk_clipboard_set_with_data(clipboard, targets, count, ClipboardData, ClearClipboard, content);
    gtk_target_table_free(targets, count);
    gtk_target_list_unref(list);
    if (!set) {
      ClearClipboard(clipboard, content);
      Failure(call, "CLIPBOARD_WRITE_FAILED", "Unable to own the clipboard"); return;
    }
    gtk_clipboard_set_can_store(clipboard, nullptr, 0);
    Success(call);
  }

  void ShowNotification(FlValue* args) {
    if (!notifications) return;
    const std::string title = StringArg(args, "title");
    const std::string body = StringArg(args, "body");
    const std::string conversation = StringArg(args, "conversationId");
    g_autoptr(GNotification) notification = g_notification_new(title.c_str());
    g_notification_set_body(notification, body.c_str());
    g_autoptr(GIcon) icon = g_themed_icon_new(APPLICATION_ID);
    g_notification_set_icon(notification, icon);
    if (!conversation.empty()) {
      g_notification_set_default_action_and_target(notification, "app.open-conversation", "s", conversation.c_str());
      g_notification_add_button_with_target(notification, "Open", "app.open-conversation", "s", conversation.c_str());
    } else {
      g_notification_set_default_action(notification, "app.show");
    }
    const std::string id = conversation.empty() ? "incoming" : conversation;
    g_application_send_notification(G_APPLICATION(application), id.c_str(), notification);
  }

  static void ClipboardChanged(GtkClipboard*, GdkEventOwnerChange*, gpointer data) {
    auto* self = static_cast<NekoPlatform*>(data);
    ++self->clipboard_revision;
    self->composer_portal_cache.clear();
    self->Emit("clipboardChanged");
  }

  static void NetworkChanged(GNetworkMonitor*, gboolean, gpointer data) {
    static_cast<NekoPlatform*>(data)->Emit("networkChanged");
  }

  void MenuAction(gint id) {
    if (id == 1) Present();
    if (id == 2 && active_transfers > 0) Emit("trayPauseAllTransfers");
    if (id == 3) { Present(); Emit("traySendClipboard"); }
    if (id == 4) RequestExit();
  }

  GVariant* Layout(gint parent = 0, gint depth = -1) {
    if (parent == 1) return MenuNode(1, "Show NekoSend", true, APPLICATION_ID);
    if (parent == 2) return MenuNode(2, "Pause transfers", active_transfers > 0, "media-playback-pause");
    if (parent == 3) return MenuNode(3, "Send clipboard", true, "edit-paste");
    if (parent == 4) return MenuNode(4, "Quit", true, "application-exit");
    if (parent != 0) return nullptr;
    GVariantBuilder children;
    g_variant_builder_init(&children, G_VARIANT_TYPE("av"));
    if (depth != 0) {
      for (gint id = 1; id <= 4; ++id) g_variant_builder_add(&children, "v", Layout(id));
    }
    GVariantBuilder properties;
    g_variant_builder_init(&properties, G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(&properties, "{sv}", "children-display", g_variant_new_string("submenu"));
    return g_variant_new("(i@a{sv}@av)", 0, g_variant_builder_end(&properties), g_variant_builder_end(&children));
  }

  GVariant* Properties(gint id, GVariant* requested_names = nullptr) {
    g_autoptr(GVariant) node = Layout(id, 0);
    if (node == nullptr) return nullptr;
    GVariant* properties = g_variant_get_child_value(node, 1);
    if (requested_names == nullptr || g_variant_n_children(requested_names) == 0) return properties;
    GVariantBuilder filtered;
    g_variant_builder_init(&filtered, G_VARIANT_TYPE_VARDICT);
    for (gsize i = 0; i < g_variant_n_children(requested_names); ++i) {
      g_autoptr(GVariant) key = g_variant_get_child_value(requested_names, i);
      const char* name = g_variant_get_string(key, nullptr);
      g_autoptr(GVariant) value = g_variant_lookup_value(properties, name, nullptr);
      if (value != nullptr) g_variant_builder_add(&filtered, "{sv}", name, value);
    }
    g_variant_unref(properties);
    return g_variant_builder_end(&filtered);
  }

  static void TrayMethod(GDBusConnection*, const gchar*, const gchar*, const gchar*,
                         const gchar* method, GVariant*, GDBusMethodInvocation* invocation,
                         gpointer data) {
    auto* self = static_cast<NekoPlatform*>(data);
    if (g_str_equal(method, "Activate") || g_str_equal(method, "SecondaryActivate")) self->Present();
    g_dbus_method_invocation_return_value(invocation, nullptr);
  }

  static GVariant* TrayProperty(GDBusConnection*, const gchar*, const gchar*, const gchar*,
                                const gchar* property, GError**, gpointer) {
    if (g_str_equal(property, "Category")) return g_variant_new_string("ApplicationStatus");
    if (g_str_equal(property, "Id") || g_str_equal(property, "Title")) return g_variant_new_string("NekoSend");
    if (g_str_equal(property, "Status")) return g_variant_new_string("Active");
    if (g_str_equal(property, "WindowId")) return g_variant_new_uint32(0);
    if (g_str_equal(property, "ItemIsMenu")) return g_variant_new_boolean(FALSE);
    if (g_str_equal(property, "Menu")) return g_variant_new_object_path("/MenuBar");
    if (g_str_has_suffix(property, "IconName")) return g_variant_new_string(g_str_equal(property, "IconName") ? APPLICATION_ID : "");
    if (g_str_has_suffix(property, "IconPixmap")) return g_variant_new_array(G_VARIANT_TYPE("(iiay)"), nullptr, 0);
    if (g_str_equal(property, "ToolTip")) {
      return g_variant_new("(s@a(iiay)ss)", APPLICATION_ID,
          g_variant_new_array(G_VARIANT_TYPE("(iiay)"), nullptr, 0),
          "NekoSend", "Local network chat and file transfer");
    }
    return nullptr;
  }

  static GVariant* MenuProperty(GDBusConnection*, const gchar*, const gchar*, const gchar*,
                                const gchar* property, GError**, gpointer) {
    if (g_str_equal(property, "Version")) return g_variant_new_uint32(3);
    if (g_str_equal(property, "TextDirection")) return g_variant_new_string("ltr");
    if (g_str_equal(property, "Status")) return g_variant_new_string("normal");
    if (g_str_equal(property, "IconThemePath")) return g_variant_new_strv(nullptr, 0);
    return nullptr;
  }

  static void MenuMethod(GDBusConnection*, const gchar*, const gchar*, const gchar*,
                         const gchar* method, GVariant* parameters,
                         GDBusMethodInvocation* invocation, gpointer data) {
    auto* self = static_cast<NekoPlatform*>(data);
    if (g_str_equal(method, "GetLayout")) {
      gint parent;
      gint depth;
      g_variant_get_child(parameters, 0, "i", &parent);
      g_variant_get_child(parameters, 1, "i", &depth);
      GVariant* node = self->Layout(parent, depth);
      if (node == nullptr) {
        g_dbus_method_invocation_return_dbus_error(invocation, "com.canonical.dbusmenu.Error.InvalidMenu", "Unknown menu item");
      } else {
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(u@(ia{sv}av))", self->menu_revision, node));
      }
    } else if (g_str_equal(method, "GetGroupProperties")) {
      g_autoptr(GVariant) ids = g_variant_get_child_value(parameters, 0);
      g_autoptr(GVariant) names = g_variant_get_child_value(parameters, 1);
      GVariantBuilder result;
      g_variant_builder_init(&result, G_VARIANT_TYPE("a(ia{sv})"));
      const gsize count = g_variant_n_children(ids);
      for (gsize i = 0; i < (count == 0 ? 5 : count); ++i) {
        gint id = static_cast<gint>(i);
        if (count != 0) g_variant_get_child(ids, i, "i", &id);
        GVariant* properties = self->Properties(id, names);
        if (properties != nullptr) g_variant_builder_add(&result, "(i@a{sv})", id, properties);
      }
      g_dbus_method_invocation_return_value(invocation, g_variant_new("(@a(ia{sv}))", g_variant_builder_end(&result)));
    } else if (g_str_equal(method, "GetProperty")) {
      gint id;
      const gchar* name;
      g_variant_get(parameters, "(i&s)", &id, &name);
      g_autoptr(GVariant) properties = self->Properties(id);
      g_autoptr(GVariant) value = properties == nullptr ? nullptr : g_variant_lookup_value(properties, name, nullptr);
      if (value == nullptr) {
        g_dbus_method_invocation_return_dbus_error(invocation, "com.canonical.dbusmenu.Error.InvalidProperty", "Unknown menu property");
      } else {
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(v)", value));
      }
    } else if (g_str_equal(method, "AboutToShow")) {
      g_dbus_method_invocation_return_value(invocation, g_variant_new("(b)", FALSE));
    } else if (g_str_equal(method, "AboutToShowGroup")) {
      g_dbus_method_invocation_return_value(invocation, g_variant_new("(@ai@ai)",
          g_variant_new_array(G_VARIANT_TYPE_INT32, nullptr, 0),
          g_variant_new_array(G_VARIANT_TYPE_INT32, nullptr, 0)));
    } else if (g_str_equal(method, "Event")) {
      gint id;
      const gchar* event;
      GVariant* value;
      guint timestamp;
      g_variant_get(parameters, "(i&s@vu)", &id, &event, &value, &timestamp);
      g_variant_unref(value);
      if (g_str_equal(event, "clicked")) self->MenuAction(id);
      g_dbus_method_invocation_return_value(invocation, nullptr);
    } else if (g_str_equal(method, "EventGroup")) {
      g_autoptr(GVariant) events = g_variant_get_child_value(parameters, 0);
      for (gsize i = 0; i < g_variant_n_children(events); ++i) {
        g_autoptr(GVariant) event = g_variant_get_child_value(events, i);
        gint id;
        const gchar* name;
        GVariant* value;
        guint timestamp;
        g_variant_get(event, "(i&s@vu)", &id, &name, &value, &timestamp);
        g_variant_unref(value);
        if (g_str_equal(name, "clicked")) self->MenuAction(id);
      }
      g_dbus_method_invocation_return_value(invocation, g_variant_new("(@ai)", g_variant_new_array(G_VARIANT_TYPE_INT32, nullptr, 0)));
    } else {
      g_dbus_method_invocation_return_dbus_error(invocation, "org.freedesktop.DBus.Error.UnknownMethod", "Unknown menu method");
    }
  }

  void RegisterTray() {
    if (bus == nullptr || !tray_registration) return;
    g_dbus_connection_call(bus, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher", "RegisterStatusNotifierItem", g_variant_new("(s)", APPLICATION_ID),
        nullptr, G_DBUS_CALL_FLAGS_NONE, 3000, cancellable, nullptr, nullptr);
  }

  void SetupTray() {
    tray_info = g_dbus_node_info_new_for_xml(kTrayXml, nullptr);
    menu_info = g_dbus_node_info_new_for_xml(kMenuXml, nullptr);
    static const GDBusInterfaceVTable tray_table = {TrayMethod, TrayProperty, nullptr, {nullptr}};
    static const GDBusInterfaceVTable menu_table = {MenuMethod, MenuProperty, nullptr, {nullptr}};
    tray_registration = g_dbus_connection_register_object(bus, "/StatusNotifierItem", tray_info->interfaces[0], &tray_table, this, nullptr, nullptr);
    menu_registration = g_dbus_connection_register_object(bus, "/MenuBar", menu_info->interfaces[0], &menu_table, this, nullptr, nullptr);
    watcher_subscription = g_dbus_connection_signal_subscribe(bus, "org.freedesktop.DBus", "org.freedesktop.DBus",
        "NameOwnerChanged", "/org/freedesktop/DBus", "org.kde.StatusNotifierWatcher", G_DBUS_SIGNAL_FLAGS_NONE,
        [](GDBusConnection*, const gchar*, const gchar*, const gchar*, const gchar*, GVariant*, gpointer data) {
          static_cast<NekoPlatform*>(data)->RegisterTray();
        }, this, nullptr);
    RegisterTray();
  }

  static NekoPlatform* FromApplication(gpointer app) {
    return static_cast<NekoPlatform*>(g_object_get_data(G_OBJECT(app), kPlatformKey));
  }

  void FinishAutostart(bool accepted, const char* message) {
    if (autostart_call == nullptr) return;
    if (portal_subscription) {
      const guint subscription = portal_subscription;
      portal_subscription = 0;
      g_dbus_connection_signal_unsubscribe(bus, subscription);
    }
    if (accepted) {
      start_on_boot = requested_autostart;
      Success(autostart_call);
    } else {
      Failure(autostart_call, "AUTOSTART_DENIED", message);
    }
    g_clear_object(&autostart_call);
  }

  void SubscribePortal(const char* path) {
    if (portal_subscription) g_dbus_connection_signal_unsubscribe(bus, portal_subscription);
    portal_path = path;
    portal_subscription = g_dbus_connection_signal_subscribe(bus, "org.freedesktop.portal.Desktop",
        "org.freedesktop.portal.Request", "Response", path, nullptr, G_DBUS_SIGNAL_FLAGS_NONE,
        [](GDBusConnection*, const gchar*, const gchar*, const gchar*, const gchar*, GVariant* parameters, gpointer app) {
          NekoPlatform* self = FromApplication(app);
          if (self == nullptr || self->autostart_call == nullptr) return;
          guint response;
          GVariant* results;
          g_variant_get(parameters, "(u@a{sv})", &response, &results);
          gboolean allowed = FALSE;
          const bool has_result = g_variant_lookup(results, "autostart", "b", &allowed);
          g_variant_unref(results);
          self->FinishAutostart(response == 0 && has_result && static_cast<bool>(allowed) == self->requested_autostart,
                               "The desktop did not allow this autostart change");
        }, g_object_ref(application), g_object_unref);
  }

  void RequestAutostart(FlMethodCall* call, bool enabled) {
    if (bus == nullptr) { Failure(call, "AUTOSTART_UNAVAILABLE", "The desktop portal is unavailable"); return; }
    if (autostart_call != nullptr) {
      Failure(call, "SETTINGS_BUSY", "An autostart permission request is already open"); return;
    }
    autostart_call = FL_METHOD_CALL(g_object_ref(call));
    requested_autostart = enabled;
    g_autofree gchar* token = g_strdup_printf("nekosend_%08x", g_random_int());
    std::string sender = g_dbus_connection_get_unique_name(bus);
    if (!sender.empty() && sender.front() == ':') sender.erase(0, 1);
    std::replace(sender.begin(), sender.end(), '.', '_');
    const std::string expected_path = "/org/freedesktop/portal/desktop/request/" + sender + "/" + token;
    SubscribePortal(expected_path.c_str());
    GVariantBuilder options;
    g_variant_builder_init(&options, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&options, "{sv}", "handle_token", g_variant_new_string(token));
    g_variant_builder_add(&options, "{sv}", "reason", g_variant_new_string("Keep NekoSend available after login"));
    g_variant_builder_add(&options, "{sv}", "autostart", g_variant_new_boolean(enabled));
    g_variant_builder_add(&options, "{sv}", "dbus-activatable", g_variant_new_boolean(FALSE));
    const char* command[] = {"nekosend", "--background", nullptr};
    g_variant_builder_add(&options, "{sv}", "commandline", g_variant_new_strv(command, -1));
    g_dbus_connection_call(bus, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Background", "RequestBackground",
        g_variant_new("(s@a{sv})", "", g_variant_builder_end(&options)), G_VARIANT_TYPE("(o)"),
        G_DBUS_CALL_FLAGS_NONE, 10000, cancellable,
        [](GObject* source, GAsyncResult* result, gpointer app) {
          g_autoptr(GError) error = nullptr;
          g_autoptr(GVariant) response = g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), result, &error);
          NekoPlatform* self = FromApplication(app);
          if (self != nullptr && self->autostart_call != nullptr) {
            if (response == nullptr) {
              self->FinishAutostart(false, error->message);
            } else {
              const gchar* path;
              g_variant_get(response, "(&o)", &path);
              if (self->portal_path != path) self->SubscribePortal(path);
            }
          }
          g_object_unref(app);
        }, g_object_ref(application));
  }
};

NekoPlatform* neko_platform_new(GtkApplication* application, GtkWindow* window, FlEngine* engine) {
  return new NekoPlatform(application, window, engine);
}

void neko_platform_free(NekoPlatform* platform) { delete platform; }

void neko_platform_ready(NekoPlatform* platform) {
  platform->ready = true;
  if (!platform->pending_conversation.empty()) {
    g_autoptr(FlValue) id = fl_value_new_string(platform->pending_conversation.c_str());
    platform->Emit("notificationOpenConversation", id);
    platform->pending_conversation.clear();
  }
}

void neko_platform_open_conversation(NekoPlatform* platform, const char* id) {
  platform->Present();
  if (!platform->ready) { platform->pending_conversation = id; return; }
  g_autoptr(FlValue) value = fl_value_new_string(id);
  platform->Emit("notificationOpenConversation", value);
}

void neko_platform_request_exit(NekoPlatform* platform) { platform->RequestExit(); }

gboolean neko_platform_window_close(NekoPlatform* platform) {
  if (platform->close_to_tray) {
    gtk_widget_hide(GTK_WIDGET(platform->window));
  } else {
    platform->Present();
    platform->Emit("windowExitRequested");
  }
  return TRUE;
}
