/* C bridge for Interpres native macOS UI (AppKit). No third-party deps. */
#ifndef INTERPRES_GUI_H
#define INTERPRES_GUI_H

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*interpres_fn)(void *user);
typedef void (*interpres_int_fn)(void *user, int value);

typedef struct InterpresGuiCallbacks {
    void *user;
    interpres_fn on_start;
    interpres_fn on_stop;
    interpres_int_fn on_remember;
    interpres_fn on_choose_folder;
    interpres_fn on_open_folder;
    interpres_fn on_check;
    interpres_int_fn on_debug; /* 1 = debug on, 0 = off */
} InterpresGuiCallbacks;

int interpres_gui_main(InterpresGuiCallbacks callbacks);

void interpres_gui_set_status(const char *text);
void interpres_gui_set_live_text(const char *text);
void interpres_gui_append_history(const char *line);
void interpres_gui_set_folder(const char *path);
void interpres_gui_set_remember(int on);
void interpres_gui_set_debug(int on);
void interpres_gui_set_listening(int on);
void interpres_gui_set_session_file(const char *path);

int interpres_gui_pick_folder(char *buf, int buflen);

#ifdef __cplusplus
}
#endif

#endif
