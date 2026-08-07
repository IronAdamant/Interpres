/*
 * Interpres native macOS UI — AppKit only (system frameworks).
 * Large high-contrast controls for Deaf / hard-of-hearing users.
 * Light / dark palettes match Windows (src/theme.rs + gui_win.rs).
 */
#import <Cocoa/Cocoa.h>
#include "interpres_gui.h"
#include <string.h>

static InterpresGuiCallbacks g_cbs;
static NSTextField *g_title;
static NSTextField *g_subtitle;
static NSTextField *g_status;
static NSTextField *g_statusLbl;
static NSTextField *g_liveLbl;
static NSTextField *g_histLbl;
static NSTextView *g_live;
static NSTextView *g_history;
static NSScrollView *g_liveScroll;
static NSScrollView *g_histScroll;
static NSTextField *g_folder;
static NSTextField *g_session;
static NSButton *g_startBtn;
static NSButton *g_stopBtn;
static NSButton *g_rememberBtn;
static NSButton *g_debugBtn;
static NSButton *g_themeBtn;
static NSButton *g_folderBtn;
static NSButton *g_openBtn;
static NSButton *g_checkBtn;
static NSWindow *g_window;
/* 0 = system, 1 = light, 2 = dark — mirrors ThemeMode in Rust */
static int g_theme_mode = 0;

/* Shared tokens with src/theme.rs PALETTE_DARK / PALETTE_LIGHT */
static BOOL effectiveIsDark(void) {
    if (g_theme_mode == 1)
        return NO;
    if (g_theme_mode == 2)
        return YES;
    NSAppearance *a = [NSApp effectiveAppearance];
    if (!a) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        a = [NSAppearance currentAppearance];
#pragma clang diagnostic pop
    }
    NSAppearanceName name =
        [a bestMatchFromAppearancesWithNames:@[ NSAppearanceNameAqua, NSAppearanceNameDarkAqua ]];
    return [name isEqualToString:NSAppearanceNameDarkAqua];
}

static NSColor *bgColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedRed:0.07 green:0.08 blue:0.10 alpha:1.0];
    return [NSColor colorWithCalibratedRed:0.96 green:0.96 blue:0.97 alpha:1.0];
}
static NSColor *panelColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedRed:0.12 green:0.13 blue:0.16 alpha:1.0];
    return [NSColor colorWithCalibratedRed:1.0 green:1.0 blue:1.0 alpha:1.0];
}
static NSColor *accentColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedRed:0.20 green:0.75 blue:0.55 alpha:1.0];
    return [NSColor colorWithCalibratedRed:0.10 green:0.61 blue:0.43 alpha:1.0];
}
static NSColor *textColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedWhite:0.95 alpha:1.0];
    return [NSColor colorWithCalibratedRed:0.10 green:0.11 blue:0.13 alpha:1.0];
}
static NSColor *mutedColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
    return [NSColor colorWithCalibratedRed:0.36 green:0.39 blue:0.44 alpha:1.0];
}
static NSColor *buttonColor(void) {
    if (effectiveIsDark())
        return [NSColor colorWithCalibratedRed:0.18 green:0.19 blue:0.22 alpha:1.0];
    return [NSColor colorWithCalibratedRed:0.90 green:0.91 blue:0.93 alpha:1.0];
}

static void styleButton(NSButton *b) {
    if (!b)
        return;
    [b setWantsLayer:YES];
    if (b.layer) {
        b.layer.backgroundColor = [buttonColor() CGColor];
        b.layer.cornerRadius = 8.0;
        b.layer.borderWidth = 1.0;
        b.layer.borderColor = [[mutedColor() colorWithAlphaComponent:0.35] CGColor];
    }
    /* Content tint for title text on modern macOS */
    if ([b respondsToSelector:@selector(setContentTintColor:)]) {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wpartial-availability"
        [b setContentTintColor:textColor()];
#pragma clang diagnostic pop
    }
    NSMutableAttributedString *attr = [[NSMutableAttributedString alloc]
        initWithString:[b title] ?: @""];
    NSDictionary *attrs = @{
        NSForegroundColorAttributeName : textColor(),
        NSFontAttributeName : [NSFont boldSystemFontOfSize:18.0]
    };
    [attr setAttributes:attrs range:NSMakeRange(0, [attr length])];
    [b setAttributedTitle:attr];
}

static void applyThemeToControls(void) {
    if (g_window)
        [g_window setBackgroundColor:bgColor()];

    void (^paintLabel)(NSTextField *, NSColor *) = ^(NSTextField *t, NSColor *c) {
      if (t)
          [t setTextColor:c];
    };
    paintLabel(g_title, textColor());
    paintLabel(g_subtitle, mutedColor());
    paintLabel(g_statusLbl, textColor());
    paintLabel(g_liveLbl, textColor());
    paintLabel(g_histLbl, textColor());
    paintLabel(g_status, mutedColor());
    paintLabel(g_folder, mutedColor());
    paintLabel(g_session, mutedColor());

    if (g_live) {
        [g_live setBackgroundColor:panelColor()];
        [g_live setTextColor:textColor()];
        [g_live setInsertionPointColor:textColor()];
    }
    if (g_history) {
        [g_history setBackgroundColor:panelColor()];
        [g_history setTextColor:textColor()];
        [g_history setInsertionPointColor:textColor()];
    }
    if (g_liveScroll) {
        [g_liveScroll setBackgroundColor:panelColor()];
        [[g_liveScroll contentView] setBackgroundColor:panelColor()];
    }
    if (g_histScroll) {
        [g_histScroll setBackgroundColor:panelColor()];
        [[g_histScroll contentView] setBackgroundColor:panelColor()];
    }

    styleButton(g_startBtn);
    styleButton(g_stopBtn);
    styleButton(g_rememberBtn);
    styleButton(g_debugBtn);
    styleButton(g_themeBtn);
    styleButton(g_folderBtn);
    styleButton(g_openBtn);
    styleButton(g_checkBtn);

    if (g_themeBtn) {
        NSString *title = @"Theme: System";
        if (g_theme_mode == 1)
            title = @"Theme: Light";
        else if (g_theme_mode == 2)
            title = @"Theme: Dark";
        [g_themeBtn setTitle:title];
        styleButton(g_themeBtn);
    }

    (void)accentColor; /* reserved for future focus/primary affordances */
    if (g_window)
        [g_window displayIfNeeded];
}

static void applyForcedAppearance(void) {
    if (g_theme_mode == 1) {
        [NSApp setAppearance:[NSAppearance appearanceNamed:NSAppearanceNameAqua]];
    } else if (g_theme_mode == 2) {
        [NSApp setAppearance:[NSAppearance appearanceNamed:NSAppearanceNameDarkAqua]];
    } else {
        [NSApp setAppearance:nil]; /* follow system */
    }
    applyThemeToControls();
}

static NSButton *makeButton(NSString *title, id target, SEL action, NSRect frame) {
    NSButton *b = [[NSButton alloc] initWithFrame:frame];
    [b setTitle:title];
    [b setBezelStyle:NSBezelStyleRegularSquare];
    [b setFont:[NSFont boldSystemFontOfSize:18.0]];
    [b setTarget:target];
    [b setAction:action];
    [b setWantsLayer:YES];
    styleButton(b);
    return b;
}

static NSTextField *makeLabel(NSString *text, NSRect frame, CGFloat size, BOOL bold) {
    NSTextField *t = [[NSTextField alloc] initWithFrame:frame];
    [t setStringValue:text];
    [t setBezeled:NO];
    [t setDrawsBackground:NO];
    [t setEditable:NO];
    [t setSelectable:YES];
    [t setTextColor:textColor()];
    [t setFont:(bold ? [NSFont boldSystemFontOfSize:size] : [NSFont systemFontOfSize:size])];
    return t;
}

static NSScrollView *makeScrollText(NSRect frame, NSTextView **outView, CGFloat fontSize) {
    NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:frame];
    [scroll setHasVerticalScroller:YES];
    [scroll setBorderType:NSLineBorder];
    [scroll setDrawsBackground:YES];
    [scroll setBackgroundColor:panelColor()];
    NSTextView *tv = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
    [tv setMinSize:NSMakeSize(0.0, frame.size.height)];
    [tv setMaxSize:NSMakeSize(FLT_MAX, FLT_MAX)];
    [tv setVerticallyResizable:YES];
    [tv setHorizontallyResizable:NO];
    [tv setAutoresizingMask:NSViewWidthSizable];
    [[tv textContainer] setContainerSize:NSMakeSize(frame.size.width, FLT_MAX)];
    [[tv textContainer] setWidthTracksTextView:YES];
    [tv setEditable:NO];
    [tv setSelectable:YES];
    [tv setBackgroundColor:panelColor()];
    [tv setTextColor:textColor()];
    [tv setFont:[NSFont systemFontOfSize:fontSize]];
    [tv setString:@""];
    [scroll setDocumentView:tv];
    *outView = tv;
    return scroll;
}

@interface InterpresAppDelegate : NSObject <NSApplicationDelegate>
@end

@implementation InterpresAppDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)note {
    (void)note;
    NSRect screen = [[NSScreen mainScreen] visibleFrame];
    CGFloat w = 920, h = 700;
    NSRect frame = NSMakeRect(NSMidX(screen) - w / 2, NSMidY(screen) - h / 2, w, h);

    g_window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                             NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    [g_window setTitle:@"Interpres — Live Captions companion"];
    [g_window setBackgroundColor:bgColor()];
    [g_window setMinSize:NSMakeSize(720, 560)];

    NSView *content = [g_window contentView];

    /* Title */
    g_title = makeLabel(@"Interpres", NSMakeRect(24, h - 56, 400, 36), 28, YES);
    g_subtitle =
        makeLabel(@"Records what Live Captions already shows — not a captioner by itself",
                  NSMakeRect(24, h - 84, 700, 24), 14, NO);
    [g_subtitle setTextColor:mutedColor()];
    [content addSubview:g_title];
    [content addSubview:g_subtitle];

    /* Big controls row — labels match Windows (symbols + copy) */
    g_startBtn = makeButton(@"▶  Start listening", self, @selector(onStart:),
                            NSMakeRect(24, h - 150, 220, 52));
    g_stopBtn = makeButton(@"■  Stop", self, @selector(onStop:),
                           NSMakeRect(256, h - 150, 140, 52));
    [g_stopBtn setEnabled:NO];
    g_rememberBtn = makeButton(@"Save to disk: OFF", self, @selector(onRemember:),
                               NSMakeRect(412, h - 150, 220, 52));
    g_folderBtn = makeButton(@"Choose folder…", self, @selector(onChooseFolder:),
                             NSMakeRect(648, h - 150, 180, 52));
    g_openBtn = makeButton(@"Open folder", self, @selector(onOpenFolder:),
                           NSMakeRect(648, h - 210, 180, 44));
    g_checkBtn = makeButton(@"Check setup", self, @selector(onCheck:),
                            NSMakeRect(24, h - 210, 180, 44));
    g_debugBtn = makeButton(@"Debug: OFF", self, @selector(onDebug:),
                            NSMakeRect(220, h - 210, 160, 44));
    g_themeBtn = makeButton(@"Theme: System", self, @selector(onTheme:),
                            NSMakeRect(396, h - 210, 180, 44));

    [content addSubview:g_startBtn];
    [content addSubview:g_stopBtn];
    [content addSubview:g_rememberBtn];
    [content addSubview:g_folderBtn];
    [content addSubview:g_openBtn];
    [content addSubview:g_checkBtn];
    [content addSubview:g_debugBtn];
    [content addSubview:g_themeBtn];

    /* Status — multi-line so Mac idle copy is not truncated mid-sentence */
    g_statusLbl = makeLabel(@"Status", NSMakeRect(24, h - 250, 100, 20), 13, YES);
    [content addSubview:g_statusLbl];
    g_status = makeLabel(@"Turn on Live Captions, then press Start listening.",
                         NSMakeRect(24, h - 300, w - 48, 48), 16, NO);
    [g_status setTextColor:mutedColor()];
    [g_status setUsesSingleLineMode:NO];
    [g_status setLineBreakMode:NSLineBreakByWordWrapping];
    [[g_status cell] setWraps:YES];
    [content addSubview:g_status];

    /* Live line — current partial/final only (not the saved list) */
    g_liveLbl = makeLabel(@"Live (now)", NSMakeRect(24, h - 330, 280, 20), 13, YES);
    [content addSubview:g_liveLbl];
    NSTextView *liveLocal = nil;
    g_liveScroll = makeScrollText(NSMakeRect(24, h - 440, w - 48, 100), &liveLocal, 22.0);
    g_live = liveLocal;
    [content addSubview:g_liveScroll];

    /* History — FINAL lines saved this session */
    g_histLbl = makeLabel(@"Session (saved lines)", NSMakeRect(24, h - 470, 320, 20), 13, YES);
    [content addSubview:g_histLbl];
    NSTextView *histLocal = nil;
    g_histScroll = makeScrollText(NSMakeRect(24, 70, w - 48, h - 550), &histLocal, 16.0);
    g_history = histLocal;
    [g_histScroll setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
    [content addSubview:g_histScroll];
    /* Folder + session path */
    g_folder = makeLabel(@"Folder: …", NSMakeRect(24, 36, w - 48, 22), 13, NO);
    [g_folder setTextColor:mutedColor()];
    [content addSubview:g_folder];
    g_session = makeLabel(@"", NSMakeRect(24, 12, w - 48, 22), 12, NO);
    [g_session setTextColor:mutedColor()];
    [content addSubview:g_session];

    applyForcedAppearance();

    /* Follow OS theme when mode is System */
    [NSApp addObserver:self
            forKeyPath:@"effectiveAppearance"
               options:NSKeyValueObservingOptionNew
               context:NULL];

    [g_window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];

    /* Tell Rust the window exists so folder/save/theme labels can be applied. */
    if (g_cbs.on_ready)
        g_cbs.on_ready(g_cbs.user);
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary *)change
                       context:(void *)context {
    (void)object;
    (void)change;
    (void)context;
    if ([keyPath isEqualToString:@"effectiveAppearance"]) {
        if (g_theme_mode == 0)
            applyThemeToControls();
    }
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    @try {
        [NSApp removeObserver:self forKeyPath:@"effectiveAppearance"];
    } @catch (__unused NSException *ex) {
    }
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return YES;
}

- (void)onStart:(id)sender {
    (void)sender;
    if (g_cbs.on_start)
        g_cbs.on_start(g_cbs.user);
}
- (void)onStop:(id)sender {
    (void)sender;
    if (g_cbs.on_stop)
        g_cbs.on_stop(g_cbs.user);
}
- (void)onRemember:(id)sender {
    (void)sender;
    int next = [[g_rememberBtn title] containsString:@"ON"] ? 0 : 1;
    if (g_cbs.on_remember)
        g_cbs.on_remember(g_cbs.user, next);
}
- (void)onChooseFolder:(id)sender {
    (void)sender;
    if (g_cbs.on_choose_folder)
        g_cbs.on_choose_folder(g_cbs.user);
}
- (void)onOpenFolder:(id)sender {
    (void)sender;
    if (g_cbs.on_open_folder)
        g_cbs.on_open_folder(g_cbs.user);
}
- (void)onCheck:(id)sender {
    (void)sender;
    if (g_cbs.on_check)
        g_cbs.on_check(g_cbs.user);
}
- (void)onDebug:(id)sender {
    (void)sender;
    int next = [[g_debugBtn title] containsString:@"ON"] ? 0 : 1;
    if (g_cbs.on_debug)
        g_cbs.on_debug(g_cbs.user, next);
}
- (void)onTheme:(id)sender {
    (void)sender;
    /* Cycle System → Light → Dark → System */
    g_theme_mode = (g_theme_mode + 1) % 3;
    applyForcedAppearance();
    if (g_cbs.on_theme)
        g_cbs.on_theme(g_cbs.user, g_theme_mode);
}

@end

static void on_main(void (^block)(void)) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_async(dispatch_get_main_queue(), block);
    }
}

int interpres_gui_main(InterpresGuiCallbacks callbacks) {
    g_cbs = callbacks;
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        InterpresAppDelegate *del = [[InterpresAppDelegate alloc] init];
        [NSApp setDelegate:del];
        [NSApp run];
    }
    return 0;
}

void interpres_gui_set_status(const char *text) {
    if (!text)
        text = "";
    NSString *s = [NSString stringWithUTF8String:text];
    on_main(^{
      if (g_status)
          [g_status setStringValue:s];
    });
}

static void scroll_textview_to_end(NSTextView *tv) {
    if (!tv)
        return;
    NSString *str = [tv string] ?: @"";
    NSRange end = NSMakeRange([str length], 0);
    [tv scrollRangeToVisible:end];
    NSScrollView *sv = [tv enclosingScrollView];
    if (sv) {
        NSView *doc = [sv documentView];
        if (doc) {
            NSRect docRect = [doc frame];
            NSRect clip = [[sv contentView] bounds];
            CGFloat y = NSMaxY(docRect) - NSHeight(clip);
            if (y < 0)
                y = 0;
            [[sv contentView] scrollToPoint:NSMakePoint(0, y)];
            [sv reflectScrolledClipView:[sv contentView]];
        }
    }
}

void interpres_gui_set_live_text(const char *text) {
    if (!text)
        text = "";
    NSString *s = [NSString stringWithUTF8String:text];
    on_main(^{
      if (g_live) {
          [g_live setString:s];
          scroll_textview_to_end(g_live);
      }
    });
}

void interpres_gui_append_history(const char *line) {
    if (!line)
        return;
    NSString *s = [NSString stringWithUTF8String:line];
    on_main(^{
      if (!g_history)
          return;
      NSString *cur = [g_history string] ?: @"";
      if ([cur length] == 0)
          [g_history setString:s];
      else
          [g_history setString:[cur stringByAppendingFormat:@"\n%@", s]];
      /* Always stick to the latest line */
      scroll_textview_to_end(g_history);
    });
}

void interpres_gui_clear_history(void) {
    on_main(^{
      if (g_history)
          [g_history setString:@""];
    });
}

void interpres_gui_set_folder(const char *path) {
    if (!path)
        path = "";
    NSString *s = [NSString stringWithFormat:@"Folder: %s", path];
    on_main(^{
      if (g_folder)
          [g_folder setStringValue:s];
    });
}

void interpres_gui_set_remember(int on) {
    on_main(^{
      if (!g_rememberBtn)
          return;
      if (on)
          [g_rememberBtn setTitle:@"Save to disk: ON"];
      else
          [g_rememberBtn setTitle:@"Save to disk: OFF"];
      styleButton(g_rememberBtn);
    });
}

void interpres_gui_set_debug(int on) {
    on_main(^{
      if (!g_debugBtn)
          return;
      if (on)
          [g_debugBtn setTitle:@"Debug: ON"];
      else
          [g_debugBtn setTitle:@"Debug: OFF"];
      styleButton(g_debugBtn);
    });
}

void interpres_gui_set_theme(int mode) {
    int m = (mode < 0 || mode > 2) ? 0 : mode;
    on_main(^{
      g_theme_mode = m;
      applyForcedAppearance();
    });
}

void interpres_gui_set_listening(int on) {
    on_main(^{
      if (g_startBtn)
          [g_startBtn setEnabled:!on];
      if (g_stopBtn)
          [g_stopBtn setEnabled:on];
      styleButton(g_startBtn);
      styleButton(g_stopBtn);
    });
}

void interpres_gui_set_session_file(const char *path) {
    if (!path)
        path = "";
    NSString *s = path[0] ? [NSString stringWithFormat:@"Saving to file: %s", path]
                          : @"";
    on_main(^{
      if (g_session)
          [g_session setStringValue:s];
    });
}

int interpres_gui_pick_folder(char *buf, int buflen) {
    if (!buf || buflen < 2)
        return 0;
    buf[0] = 0;
    __block int ok = 0;
    void (^pick)(void) = ^{
      NSOpenPanel *panel = [NSOpenPanel openPanel];
      [panel setCanChooseFiles:NO];
      [panel setCanChooseDirectories:YES];
      [panel setAllowsMultipleSelection:NO];
      [panel setCanCreateDirectories:YES];
      [panel setMessage:@"Choose where Interpres should save transcript files"];
      [panel setPrompt:@"Use this folder"];
      if ([panel runModal] == NSModalResponseOK) {
          NSURL *url = [[panel URLs] firstObject];
          if (url) {
              const char *p = [[url path] UTF8String];
              if (p) {
                  strncpy(buf, p, (size_t)buflen - 1);
                  buf[buflen - 1] = 0;
                  ok = 1;
              }
          }
      }
    };
    if ([NSThread isMainThread]) {
        pick();
    } else {
        dispatch_sync(dispatch_get_main_queue(), pick);
    }
    return ok;
}
