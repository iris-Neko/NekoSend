#include "my_application.h"
#include "platform_adapter.h"
#include <cstring>

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  GtkWindow* window;
  NekoPlatform* platform;
  gboolean start_hidden;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  neko_platform_ready(self->platform);
  if (!self->start_hidden) gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

static gboolean window_close_cb(GtkWidget*, GdkEvent*, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  return self->platform == nullptr ? FALSE : neko_platform_window_close(self->platform);
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  if (self->window != nullptr) {
    gtk_widget_show(GTK_WIDGET(self->window));
    gtk_window_present(self->window);
    return;
  }
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  self->window = window;
  gtk_window_set_title(window, "NekoSend");
  gtk_window_set_icon_name(window, APPLICATION_ID);
  g_signal_connect(window, "delete-event", G_CALLBACK(window_close_cb), self);

  // Use a header bar when running in GNOME as this is the common style used
  // by applications and is the setup most users will be using (e.g. Ubuntu
  // desktop).
  // If running on X and not using GNOME then just use a traditional title bar
  // in case the window manager does more exotic layout, e.g. tiling.
  // If running on Wayland assume the header bar will work (may need changing
  // if future cases occur).
  gboolean use_header_bar = TRUE;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    if (g_strcmp0(wm_name, "GNOME Shell") != 0) {
      use_header_bar = FALSE;
    }
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "NekoSend");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "NekoSend");
  }

  gtk_window_set_default_size(window, 1280, 720);
  GdkGeometry geometry = {};
  geometry.min_width = 640;
  geometry.min_height = 480;
  gtk_window_set_geometry_hints(window, nullptr, &geometry, GDK_HINT_MIN_SIZE);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  self->platform = neko_platform_new(GTK_APPLICATION(application), window, fl_view_get_engine(view));
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#F2F5F6");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

static void show_action(GSimpleAction*, GVariant*, gpointer data) {
  g_application_activate(G_APPLICATION(data));
}

static void quit_action(GSimpleAction*, GVariant*, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  if (self->platform != nullptr) {
    neko_platform_request_exit(self->platform);
  } else {
    g_application_quit(G_APPLICATION(self));
  }
}

static void open_conversation_action(GSimpleAction*, GVariant* parameter, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  const char* conversation = g_variant_get_string(parameter, nullptr);
  if (self->window == nullptr) {
    g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
    self->dart_entrypoint_arguments = g_new0(char*, 2);
    self->dart_entrypoint_arguments[0] = g_strdup_printf("openConversation:%s", conversation);
    self->start_hidden = FALSE;
    g_application_activate(G_APPLICATION(self));
  } else {
    neko_platform_open_conversation(self->platform, conversation);
  }
}

// GApplication forwards later launches to the existing process.
static int my_application_command_line(GApplication* application,
                                       GApplicationCommandLine* command_line) {
  MyApplication* self = MY_APPLICATION(application);
  gint count = 0;
  gchar** arguments = g_application_command_line_get_arguments(command_line, &count);
  const gboolean first_launch = self->window == nullptr;
  gboolean background = FALSE;
  const char* route = nullptr;
  for (gint i = 1; i < count; ++i) {
    if (g_str_equal(arguments[i], "--background")) background = TRUE;
    if (g_str_has_prefix(arguments[i], "openConversation:")) route = arguments[i] + strlen("openConversation:");
    if (g_str_equal(arguments[i], "--quit")) {
      quit_action(nullptr, nullptr, self);
      g_strfreev(arguments);
      return 0;
    }
  }
  if (first_launch) {
    self->dart_entrypoint_arguments = g_strdupv(arguments + 1);
    self->start_hidden = background && route == nullptr;
    g_application_activate(application);
  } else if (route != nullptr) {
    neko_platform_open_conversation(self->platform, route);
  } else if (!background) {
    g_application_activate(application);
  }
  g_strfreev(arguments);
  return 0;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
  const GActionEntry actions[] = {
    {"show", show_action, nullptr, nullptr, nullptr, {0, 0, 0}},
    {"open-conversation", open_conversation_action, "s", nullptr, nullptr, {0, 0, 0}},
    {"quit", quit_action, nullptr, nullptr, nullptr, {0, 0, 0}},
  };
  g_action_map_add_action_entries(G_ACTION_MAP(application), actions, G_N_ELEMENTS(actions), application);
  const char* quit_shortcuts[] = {"<Primary>q", nullptr};
  gtk_application_set_accels_for_action(GTK_APPLICATION(application), "app.quit", quit_shortcuts);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  neko_platform_free(self->platform);
  self->platform = nullptr;
  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  neko_platform_free(self->platform);
  self->platform = nullptr;
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->command_line = my_application_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);
  g_set_application_name("NekoSend");

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_HANDLES_COMMAND_LINE, nullptr));
}
