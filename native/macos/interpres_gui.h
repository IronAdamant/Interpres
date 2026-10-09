/* C bridge for Interpres native macOS UI (AppKit). No third-party deps.
 *
 * The window is a thin view: Rust (`src/gui.rs` + `src/app_view.rs`) decides what
 * every control shows and pushes it here. All functions must be called on the main
 * thread (callbacks already run there: button clicks and the ~50 ms tick). */
#ifndef INTERPRES_GUI_H
#define INTERPRES_GUI_H

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*interpres_fn)(void *user);
typedef void (*interpres_int_fn)(void *user, int value);

/* Command ids sent to on_command — mirrored in src/gui.rs (`cmd`). */
enum {
    INTERPRES_CMD_TOGGLE = 1,      /* Start / Stop recording */
    INTERPRES_CMD_ACTION = 2,      /* contextual Live Captions button */
    INTERPRES_CMD_SETTINGS = 3,    /* Settings ▾ button → Rust shows the menu */
    INTERPRES_CMD_OPEN_FILE = 4,
    INTERPRES_CMD_COPY = 5,
    INTERPRES_CMD_OPEN_FOLDER = 6,
    INTERPRES_CMD_AUTO = 7,        /* Auto-record checkbox */
    INTERPRES_CMD_PREFERENCES = 8, /* app menu "Settings…" (⌘,) */
};

/* Banner tones — mirrored in src/app_view.rs (`Tone::as_int`). */
enum {
    INTERPRES_TONE_NEUTRAL = 0,
    INTERPRES_TONE_RECORDING = 1,
    INTERPRES_TONE_WAITING = 2,
    INTERPRES_TONE_PROBLEM = 3,
    INTERPRES_TONE_ACTION = 4,
};

typedef struct InterpresGuiCallbacks {
    void *user;
    interpres_int_fn on_command; /* INTERPRES_CMD_* */
    interpres_fn on_tick;        /* every ~50 ms on the main thread */
    interpres_fn on_ready;       /* window created */
    interpres_fn on_quit;        /* app is quitting: save the last words, stop */
    interpres_fn on_appearance;  /* OS light/dark changed */
} InterpresGuiCallbacks;

/* One Settings-menu row. id 0 = separator. */
typedef struct InterpresMenuItem {
    int id;
    const char *title;
    int checked;
    int enabled;
} InterpresMenuItem;

/* Runs the app until it quits. start_minimized: open in the Dock without taking focus. */
int interpres_gui_main(InterpresGuiCallbacks callbacks, int start_minimized);

void interpres_gui_set_title(const char *text);
void interpres_gui_set_banner(const char *head, const char *guidance, int tone);
/* recording: 1 → red Stop style, 0 → green Start style. */
void interpres_gui_set_toggle(const char *label, int enabled, int recording);
/* NULL or "" hides the contextual button. */
void interpres_gui_set_action(const char *label);
void interpres_gui_set_checks(const char *text);
void interpres_gui_set_detail(const char *text);
void interpres_gui_set_footer(const char *text);
void interpres_gui_set_enabled(int open_file, int copy);
void interpres_gui_set_auto(int on);
/* UI appearance: 0 = system, 1 = light, 2 = dark. Does not affect capture. */
void interpres_gui_set_theme(int mode);
/* 1 if the window currently looks dark. */
int interpres_gui_is_dark(void);
/* Replace the transcript from UTF-16 offset `start` to the end with `tail`. Keeps the
 * reader's scroll position unless they were at the bottom. */
void interpres_gui_transcript_replace_tail(long start, const char *tail);

/* Pop up the Settings menu under its button; returns the chosen id (0 = none). */
int interpres_gui_show_menu(const InterpresMenuItem *items, int count);
/* Ask for attention: critical = 1 bounces the Dock until focused + warning sound;
 * 0 bounces once + soft sound. */
void interpres_gui_attention(int critical);
int interpres_gui_copy_text(const char *text);
int interpres_gui_pick_folder(char *buf, int buflen);

#ifdef __cplusplus
}
#endif

#endif
