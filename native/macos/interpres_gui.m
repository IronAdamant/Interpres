/*
 * Interpres native macOS UI — AppKit only (system frameworks).
 *
 * Same layout as the Windows window (src/gui_win.rs): title + Auto-record checkbox +
 * Settings menu, a coloured status banner, one Start/Stop button plus a contextual
 * Live Captions button, a setup checklist, one transcript view (saved lines + the line
 * being spoken), and a footer with Open / Copy / Folder actions.
 *
 * This file only draws. Rust decides what every control shows (src/app_view.rs) and
 * pushes it through interpres_gui.h. Palettes mirror src/theme.rs.
 */
#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#include "interpres_gui.h"
#include <string.h>

@class IPFilledButton;

static InterpresGuiCallbacks g_cbs;
static NSWindow *g_window;
static NSTextField *g_title;
static NSButton *g_auto;
static NSButton *g_settings;
static NSView *g_banner;
static NSTextField *g_bannerHead;
static NSTextField *g_bannerDetail;
static IPFilledButton *g_toggle;
static IPFilledButton *g_action;
static NSTextField *g_checks;
static NSTextField *g_detail;
static NSTextField *g_transcriptLbl;
static NSScrollView *g_transcriptScroll;
static NSTextView *g_transcript;
static NSTextField *g_file;
static NSButton *g_openFile;
static NSButton *g_copy;
static NSButton *g_openFolder;
static NSTimer *g_timer;
/* 0 = system, 1 = light, 2 = dark — mirrors ThemeMode in Rust */
static int g_theme_mode = 0;
static int g_tone = INTERPRES_TONE_NEUTRAL;
static int g_recording = 0;
static int g_quit_sent = 0;
/* Item chosen in the Settings menu while it is open. */
static int g_menu_choice = 0;

/* ---------- palette (src/theme.rs) ---------- */

static BOOL effectiveIsDark(void) {
    if (g_theme_mode == 1)
        return NO;
    if (g_theme_mode == 2)
        return YES;
    NSAppearance *a = [NSApp effectiveAppearance];
    NSAppearanceName name =
        [a bestMatchFromAppearancesWithNames:@[ NSAppearanceNameAqua, NSAppearanceNameDarkAqua ]];
    return [name isEqualToString:NSAppearanceNameDarkAqua];
}

static NSColor *rgb(CGFloat r, CGFloat g, CGFloat b) {
    return [NSColor colorWithSRGBRed:r green:g blue:b alpha:1.0];
}
static NSColor *bgColor(void) {
    return effectiveIsDark() ? rgb(0.07, 0.08, 0.10) : rgb(0.96, 0.96, 0.97);
}
static NSColor *panelColor(void) {
    return effectiveIsDark() ? rgb(0.12, 0.13, 0.16) : rgb(1.0, 1.0, 1.0);
}
static NSColor *textColor(void) {
    return effectiveIsDark() ? rgb(0.95, 0.95, 0.95) : rgb(0.10, 0.11, 0.13);
}
static NSColor *mutedColor(void) {
    return effectiveIsDark() ? rgb(0.65, 0.65, 0.65) : rgb(0.36, 0.39, 0.44);
}
static NSColor *borderColor(void) {
    return effectiveIsDark() ? rgb(0.28, 0.30, 0.34) : rgb(0.78, 0.80, 0.83);
}
/* Fixed status colours — same RGB as COL_* in gui_win.rs. */
static NSColor *colRecording(void) { return rgb(28 / 255.0, 128 / 255.0, 72 / 255.0); }
static NSColor *colWaiting(void) { return rgb(37 / 255.0, 99 / 255.0, 160 / 255.0); }
static NSColor *colProblem(void) { return rgb(176 / 255.0, 36 / 255.0, 36 / 255.0); }
static NSColor *colAction(void) { return rgb(166 / 255.0, 92 / 255.0, 0); }

static NSColor *toneColor(int tone) {
    switch (tone) {
    case INTERPRES_TONE_RECORDING: return colRecording();
    case INTERPRES_TONE_WAITING: return colWaiting();
    case INTERPRES_TONE_PROBLEM: return colProblem();
    case INTERPRES_TONE_ACTION: return colAction();
    default: return panelColor();
    }
}

static NSString *str(const char *s) {
    if (!s)
        return @"";
    NSString *v = [NSString stringWithUTF8String:s];
    return v ?: @"";
}

/* ---------- views ---------- */

/* Top-left origin, like the Windows layout. */
@interface IPFlippedView : NSView
@end
@implementation IPFlippedView
- (BOOL)isFlipped {
    return YES;
}
@end

/* Rounded, filled button with white text (Start/Stop and the Live Captions action). */
@interface IPFilledButton : NSButton
@property(nonatomic, strong) NSColor *fill;
@end
@implementation IPFilledButton
- (void)drawRect:(NSRect)dirty {
    (void)dirty;
    NSRect r = NSInsetRect(self.bounds, 0.5, 0.5);
    NSColor *c = self.fill ?: colRecording();
    if (self.isHighlighted)
        c = [c blendedColorWithFraction:0.15 ofColor:NSColor.blackColor];
    if (!self.isEnabled)
        c = [c colorWithAlphaComponent:0.55];
    [c setFill];
    [[NSBezierPath bezierPathWithRoundedRect:r xRadius:9 yRadius:9] fill];
    NSMutableParagraphStyle *ps = [[NSMutableParagraphStyle alloc] init];
    ps.alignment = NSTextAlignmentCenter;
    NSDictionary *attrs = @{
        NSFontAttributeName : [NSFont systemFontOfSize:17 weight:NSFontWeightSemibold],
        NSForegroundColorAttributeName : NSColor.whiteColor,
        NSParagraphStyleAttributeName : ps,
    };
    NSString *t = self.title ?: @"";
    NSSize sz = [t sizeWithAttributes:attrs];
    NSRect tr = NSMakeRect(0, (NSHeight(self.bounds) - sz.height) / 2, NSWidth(self.bounds), sz.height);
    [t drawInRect:tr withAttributes:attrs];
}
- (BOOL)isFlipped {
    return YES;
}
/* Keyboard focus ring follows the rounded shape (the default is a standard-height
 * button bar across the middle). */
- (NSRect)focusRingMaskBounds {
    return self.bounds;
}
- (void)drawFocusRingMask {
    [[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(self.bounds, 0.5, 0.5) xRadius:9 yRadius:9] fill];
}
@end

static NSTextField *makeLabel(NSString *text, CGFloat size, NSFontWeight weight) {
    NSTextField *t = [NSTextField labelWithString:text];
    t.font = [NSFont systemFontOfSize:size weight:weight];
    t.textColor = textColor();
    t.lineBreakMode = NSLineBreakByTruncatingTail;
    t.selectable = NO;
    return t;
}

static NSButton *makePlainButton(NSString *title, id target, SEL action) {
    NSButton *b = [NSButton buttonWithTitle:title target:target action:action];
    b.bezelStyle = NSBezelStyleRounded;
    b.controlSize = NSControlSizeLarge;
    b.font = [NSFont systemFontOfSize:15];
    return b;
}

static IPFilledButton *makeFilledButton(NSString *title, id target, SEL action) {
    IPFilledButton *b = [[IPFilledButton alloc] initWithFrame:NSZeroRect];
    b.title = title;
    b.bordered = NO;
    b.target = target;
    b.action = action;
    b.wantsLayer = YES;
    return b;
}

static NSDictionary *transcriptAttrs(void) {
    NSMutableParagraphStyle *ps = [[NSMutableParagraphStyle alloc] init];
    ps.paragraphSpacing = 4;
    ps.headIndent = 0;
    return @{
        NSFontAttributeName : [NSFont systemFontOfSize:18],
        NSForegroundColorAttributeName : textColor(),
        NSParagraphStyleAttributeName : ps,
    };
}

/* ---------- theme ---------- */

static void paintBanner(void) {
    if (!g_banner)
        return;
    g_banner.layer.backgroundColor = toneColor(g_tone).CGColor;
    BOOL neutral = (g_tone == INTERPRES_TONE_NEUTRAL);
    g_banner.layer.borderWidth = neutral ? 1.0 : 0.0;
    g_banner.layer.borderColor = borderColor().CGColor;
    NSColor *fg = neutral ? textColor() : NSColor.whiteColor;
    g_bannerHead.textColor = fg;
    g_bannerDetail.textColor = neutral ? mutedColor() : [NSColor.whiteColor colorWithAlphaComponent:0.92];
}

static void applyTheme(void) {
    if (!g_window)
        return;
    g_window.backgroundColor = bgColor();
    g_window.contentView.layer.backgroundColor = bgColor().CGColor;
    g_title.textColor = textColor();
    g_checks.textColor = textColor();
    g_detail.textColor = mutedColor();
    g_transcriptLbl.textColor = textColor();
    g_file.textColor = mutedColor();
    g_transcript.backgroundColor = panelColor();
    g_transcriptScroll.backgroundColor = panelColor();
    g_transcriptScroll.wantsLayer = YES;
    g_transcriptScroll.layer.borderColor = borderColor().CGColor;
    g_transcriptScroll.layer.borderWidth = 1.0;
    g_transcriptScroll.layer.cornerRadius = 6.0;
    NSTextStorage *ts = g_transcript.textStorage;
    [ts addAttribute:NSForegroundColorAttributeName value:textColor() range:NSMakeRange(0, ts.length)];
    g_transcript.typingAttributes = transcriptAttrs();
    g_toggle.fill = g_recording ? colProblem() : colRecording();
    [g_toggle setNeedsDisplay:YES];
    paintBanner();
}

static void applyAppearance(void) {
    if (g_theme_mode == 1)
        NSApp.appearance = [NSAppearance appearanceNamed:NSAppearanceNameAqua];
    else if (g_theme_mode == 2)
        NSApp.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
    else
        NSApp.appearance = nil;
    applyTheme();
}

/* ---------- layout (mirrors gui_win.rs `layout`) ---------- */

static void layoutWindow(void) {
    NSView *content = g_window.contentView;
    CGFloat w = NSWidth(content.bounds), h = NSHeight(content.bounds);
    CGFloat m = 20, bw = MAX(w - m * 2, 200);

    g_title.frame = NSMakeRect(m, 12, 300, 38);
    g_settings.frame = NSMakeRect(w - m - 140, 14, 140, 34);
    g_auto.frame = NSMakeRect(w - m - 140 - 16 - 260, 18, 260, 26);

    g_banner.frame = NSMakeRect(m, 62, bw, 78);
    g_bannerHead.frame = NSMakeRect(18, 10, bw - 36, 30);
    g_bannerDetail.frame = NSMakeRect(18, 44, bw - 36, 24);

    CGFloat by = 156;
    g_toggle.frame = NSMakeRect(m, by, 240, 48);
    g_action.frame = NSMakeRect(m + 256, by, 240, 48);
    g_checks.frame = NSMakeRect(m, by + 62, bw, 22);
    g_detail.frame = NSMakeRect(m, by + 88, bw, 20);

    CGFloat ty = by + 120;
    g_transcriptLbl.frame = NSMakeRect(m, ty, 200, 22);
    CGFloat footer_h = 58;
    CGFloat th = MAX(h - (ty + 26) - footer_h, 80);
    g_transcriptScroll.frame = NSMakeRect(m, ty + 26, bw, th);

    CGFloat fy = h - footer_h + 12;
    CGFloat btn_w[3] = {170, 110, 130};
    CGFloat gap = 10;
    CGFloat buttons_w = btn_w[0] + btn_w[1] + btn_w[2] + gap * 2;
    CGFloat x = m + bw - buttons_w;
    g_file.frame = NSMakeRect(m, fy + 8, MAX(x - m - 12, 80), 20);
    NSButton *btns[3] = {g_openFile, g_copy, g_openFolder};
    for (int i = 0; i < 3; i++) {
        btns[i].frame = NSMakeRect(x, fy, btn_w[i], 34);
        x += btn_w[i] + gap;
    }
}

/* ---------- app delegate ---------- */

@interface InterpresAppDelegate : NSObject <NSApplicationDelegate, NSWindowDelegate>
@property(nonatomic) BOOL startMinimized;
@end

@implementation InterpresAppDelegate

- (void)buildMainMenu {
    NSMenu *bar = [[NSMenu alloc] init];

    NSMenuItem *appItem = [[NSMenuItem alloc] init];
    NSMenu *app = [[NSMenu alloc] initWithTitle:@"Interpres"];
    [app addItemWithTitle:@"About Interpres"
                   action:@selector(orderFrontStandardAboutPanel:)
            keyEquivalent:@""];
    [app addItem:[NSMenuItem separatorItem]];
    NSMenuItem *prefs = [app addItemWithTitle:@"Settings…" action:@selector(onPreferences:) keyEquivalent:@","];
    prefs.target = self;
    [app addItem:[NSMenuItem separatorItem]];
    [app addItemWithTitle:@"Hide Interpres" action:@selector(hide:) keyEquivalent:@"h"];
    NSMenuItem *others = [app addItemWithTitle:@"Hide Others"
                                        action:@selector(hideOtherApplications:)
                                 keyEquivalent:@"h"];
    others.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagOption;
    [app addItemWithTitle:@"Show All" action:@selector(unhideAllApplications:) keyEquivalent:@""];
    [app addItem:[NSMenuItem separatorItem]];
    [app addItemWithTitle:@"Quit Interpres" action:@selector(terminate:) keyEquivalent:@"q"];
    appItem.submenu = app;
    [bar addItem:appItem];

    NSMenuItem *editItem = [[NSMenuItem alloc] init];
    NSMenu *edit = [[NSMenu alloc] initWithTitle:@"Edit"];
    [edit addItemWithTitle:@"Copy" action:@selector(copy:) keyEquivalent:@"c"];
    [edit addItemWithTitle:@"Select All" action:@selector(selectAll:) keyEquivalent:@"a"];
    editItem.submenu = edit;
    [bar addItem:editItem];

    NSMenuItem *winItem = [[NSMenuItem alloc] init];
    NSMenu *win = [[NSMenu alloc] initWithTitle:@"Window"];
    [win addItemWithTitle:@"Minimize" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
    [win addItemWithTitle:@"Close" action:@selector(performClose:) keyEquivalent:@"w"];
    winItem.submenu = win;
    [bar addItem:winItem];
    NSApp.windowsMenu = win;

    NSApp.mainMenu = bar;
}

- (void)applicationDidFinishLaunching:(NSNotification *)note {
    (void)note;
    [self buildMainMenu];

    NSRect screen = NSScreen.mainScreen.visibleFrame;
    CGFloat w = 1000, h = 780;
    w = MIN(w, NSWidth(screen) - 40);
    h = MIN(h, NSHeight(screen) - 40);
    NSRect frame = NSMakeRect(NSMidX(screen) - w / 2, NSMidY(screen) - h / 2, w, h);
    g_window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                             NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    g_window.title = @"Interpres";
    g_window.minSize = NSMakeSize(860, 560);
    g_window.delegate = self;
    g_window.releasedWhenClosed = NO;
    g_window.frameAutosaveName = @"InterpresMainWindow";

    IPFlippedView *content = [[IPFlippedView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
    content.wantsLayer = YES;
    g_window.contentView = content;

    g_title = makeLabel(@"Interpres", 28, NSFontWeightBold);
    g_auto = [NSButton checkboxWithTitle:@"Auto-record when sound plays"
                                  target:self
                                  action:@selector(onAuto:)];
    g_auto.font = [NSFont systemFontOfSize:16];
    g_settings = makePlainButton(@"Settings  ▾", self, @selector(onSettings:));

    g_banner = [[IPFlippedView alloc] initWithFrame:NSZeroRect];
    g_banner.wantsLayer = YES;
    g_banner.layer.cornerRadius = 10;
    g_bannerHead = makeLabel(@"", 21, NSFontWeightSemibold);
    g_bannerDetail = makeLabel(@"", 16, NSFontWeightRegular);
    [g_banner addSubview:g_bannerHead];
    [g_banner addSubview:g_bannerDetail];

    g_toggle = makeFilledButton(@"▶   Start recording", self, @selector(onToggle:));
    g_toggle.keyEquivalent = @"\r";
    g_action = makeFilledButton(@"Turn on Live Captions", self, @selector(onAction:));
    g_action.fill = colAction();
    g_action.hidden = YES;

    g_checks = makeLabel(@"", 16, NSFontWeightRegular);
    g_detail = makeLabel(@"", 14, NSFontWeightRegular);
    g_detail.selectable = YES;
    g_transcriptLbl = makeLabel(@"Transcript", 16, NSFontWeightSemibold);

    g_transcriptScroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(0, 0, 400, 200)];
    g_transcriptScroll.hasVerticalScroller = YES;
    g_transcriptScroll.borderType = NSNoBorder;
    g_transcriptScroll.drawsBackground = YES;
    NSSize cs = g_transcriptScroll.contentSize;
    g_transcript = [[NSTextView alloc] initWithFrame:NSMakeRect(0, 0, cs.width, cs.height)];
    g_transcript.minSize = NSMakeSize(0, cs.height);
    g_transcript.maxSize = NSMakeSize(FLT_MAX, FLT_MAX);
    g_transcript.verticallyResizable = YES;
    g_transcript.horizontallyResizable = NO;
    g_transcript.autoresizingMask = NSViewWidthSizable;
    g_transcript.textContainer.widthTracksTextView = YES;
    g_transcript.textContainerInset = NSMakeSize(8, 8);
    g_transcript.editable = NO;
    g_transcript.selectable = YES;
    g_transcript.richText = NO;
    g_transcript.typingAttributes = transcriptAttrs();
    g_transcriptScroll.documentView = g_transcript;

    g_file = makeLabel(@"", 13, NSFontWeightRegular);
    g_file.lineBreakMode = NSLineBreakByTruncatingMiddle;
    g_file.selectable = YES;
    g_openFile = makePlainButton(@"Open transcript", self, @selector(onOpenFile:));
    g_copy = makePlainButton(@"Copy all", self, @selector(onCopy:));
    g_openFolder = makePlainButton(@"Open folder", self, @selector(onOpenFolder:));
    g_openFile.enabled = NO;
    g_copy.enabled = NO;

    for (NSView *v in @[
             g_title, g_auto, g_settings, g_banner, g_toggle, g_action, g_checks, g_detail,
             g_transcriptLbl, g_transcriptScroll, g_file, g_openFile, g_copy, g_openFolder
         ])
        [content addSubview:v];

    layoutWindow();
    applyAppearance();
    [NSApp addObserver:self forKeyPath:@"effectiveAppearance" options:NSKeyValueObservingOptionNew context:NULL];

    if (g_cbs.on_ready)
        g_cbs.on_ready(g_cbs.user);
    if (g_cbs.on_tick)
        g_cbs.on_tick(g_cbs.user);

    g_timer = [NSTimer scheduledTimerWithTimeInterval:0.05
                                              repeats:YES
                                                block:^(NSTimer *t) {
                                                  (void)t;
                                                  if (g_cbs.on_tick)
                                                      g_cbs.on_tick(g_cbs.user);
                                                }];

    /* Dev aid: INTERPRES_SNAPSHOT=/path/shot.png saves a picture of the window
     * (README screenshots, checking the layout without Screen Recording permission). */
    const char *snap = getenv("INTERPRES_SNAPSHOT");
    if (snap && snap[0]) {
        NSString *path = str(snap);
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
          NSView *v = g_window.contentView;
          NSBitmapImageRep *rep = [v bitmapImageRepForCachingDisplayInRect:v.bounds];
          [v cacheDisplayInRect:v.bounds toBitmapImageRep:rep];
          NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
          [png writeToFile:path atomically:YES];
        });
    }

    if (self.startMinimized) {
        [g_window orderFront:nil];
        [g_window miniaturize:nil];
    } else {
        [g_window makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
    }
}

- (void)windowDidResize:(NSNotification *)note {
    (void)note;
    layoutWindow();
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary *)change
                       context:(void *)context {
    (void)object;
    (void)change;
    (void)context;
    if ([keyPath isEqualToString:@"effectiveAppearance"] && g_theme_mode == 0) {
        applyTheme();
        if (g_cbs.on_appearance)
            g_cbs.on_appearance(g_cbs.user);
    }
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return YES;
}

- (BOOL)applicationShouldHandleReopen:(NSApplication *)sender hasVisibleWindows:(BOOL)flag {
    (void)sender;
    if (!flag)
        [g_window makeKeyAndOrderFront:nil];
    return YES;
}

- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender {
    (void)sender;
    if (!g_quit_sent) {
        g_quit_sent = 1;
        [g_timer invalidate];
        g_timer = nil;
        /* Disappear at once; saving the last sentence can take a couple of seconds. */
        [g_window orderOut:nil];
        [CATransaction flush];
        if (g_cbs.on_quit)
            g_cbs.on_quit(g_cbs.user);
    }
    return NSTerminateNow;
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    @try {
        [NSApp removeObserver:self forKeyPath:@"effectiveAppearance"];
    } @catch (__unused NSException *ex) {
    }
}

static void sendCommand(int cmd) {
    if (g_cbs.on_command)
        g_cbs.on_command(g_cbs.user, cmd);
}

- (void)onToggle:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_TOGGLE); }
- (void)onAction:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_ACTION); }
- (void)onSettings:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_SETTINGS); }
- (void)onPreferences:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_PREFERENCES); }
- (void)onOpenFile:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_OPEN_FILE); }
- (void)onCopy:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_COPY); }
- (void)onOpenFolder:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_OPEN_FOLDER); }
- (void)onAuto:(id)sender { (void)sender; sendCommand(INTERPRES_CMD_AUTO); }
- (void)onMenuItem:(NSMenuItem *)item { g_menu_choice = (int)item.tag; }

@end

/* ---------- C API ---------- */

int interpres_gui_main(InterpresGuiCallbacks callbacks, int start_minimized) {
    g_cbs = callbacks;
    @autoreleasepool {
        [NSApplication sharedApplication];
        NSApp.activationPolicy = NSApplicationActivationPolicyRegular;
        InterpresAppDelegate *del = [[InterpresAppDelegate alloc] init];
        del.startMinimized = start_minimized != 0;
        NSApp.delegate = del;
        [NSApp run];
    }
    return 0;
}

void interpres_gui_set_title(const char *text) {
    NSString *s = str(text);
    if (g_window && ![g_window.title isEqualToString:s])
        g_window.title = s;
}

void interpres_gui_set_banner(const char *head, const char *guidance, int tone) {
    NSString *h = str(head), *d = str(guidance);
    if (![g_bannerHead.stringValue isEqualToString:h])
        g_bannerHead.stringValue = h;
    if (![g_bannerDetail.stringValue isEqualToString:d])
        g_bannerDetail.stringValue = d;
    if (tone != g_tone) {
        g_tone = tone;
        paintBanner();
    }
}

void interpres_gui_set_toggle(const char *label, int enabled, int recording) {
    NSString *s = str(label);
    BOOL dirty = NO;
    if (![g_toggle.title isEqualToString:s]) {
        g_toggle.title = s;
        dirty = YES;
    }
    if (g_toggle.enabled != (enabled != 0)) {
        g_toggle.enabled = enabled != 0;
        dirty = YES;
    }
    if (g_recording != recording) {
        g_recording = recording;
        g_toggle.fill = recording ? colProblem() : colRecording();
        dirty = YES;
    }
    if (dirty)
        [g_toggle setNeedsDisplay:YES];
}

void interpres_gui_set_action(const char *label) {
    NSString *s = str(label);
    BOOL hide = s.length == 0;
    if (g_action.hidden != hide)
        g_action.hidden = hide;
    if (!hide && ![g_action.title isEqualToString:s]) {
        g_action.title = s;
        [g_action setNeedsDisplay:YES];
    }
}

static void setLabel(NSTextField *t, const char *text) {
    NSString *s = str(text);
    if (t && ![t.stringValue isEqualToString:s])
        t.stringValue = s;
}

void interpres_gui_set_checks(const char *text) { setLabel(g_checks, text); }
void interpres_gui_set_detail(const char *text) { setLabel(g_detail, text); }
void interpres_gui_set_footer(const char *text) { setLabel(g_file, text); }

void interpres_gui_set_enabled(int open_file, int copy) {
    if (g_openFile.enabled != (open_file != 0))
        g_openFile.enabled = open_file != 0;
    if (g_copy.enabled != (copy != 0))
        g_copy.enabled = copy != 0;
}

void interpres_gui_set_auto(int on) {
    NSControlStateValue v = on ? NSControlStateValueOn : NSControlStateValueOff;
    if (g_auto.state != v)
        g_auto.state = v;
}

void interpres_gui_set_theme(int mode) {
    g_theme_mode = (mode < 0 || mode > 2) ? 0 : mode;
    applyAppearance();
}

int interpres_gui_is_dark(void) { return effectiveIsDark() ? 1 : 0; }

void interpres_gui_transcript_replace_tail(long start, const char *tail) {
    if (!g_transcript)
        return;
    NSTextStorage *ts = g_transcript.textStorage;
    NSUInteger len = ts.length;
    NSUInteger from = start < 0 ? 0 : MIN((NSUInteger)start, len);
    NSClipView *clip = g_transcriptScroll.contentView;
    NSRect visible = clip.documentVisibleRect;
    BOOL atBottom = NSMaxY(visible) >= NSHeight(g_transcript.frame) - 24;
    NSPoint keep = clip.bounds.origin;

    NSAttributedString *add = [[NSAttributedString alloc] initWithString:str(tail)
                                                              attributes:transcriptAttrs()];
    [ts beginEditing];
    [ts replaceCharactersInRange:NSMakeRange(from, len - from) withAttributedString:add];
    [ts endEditing];

    if (atBottom) {
        [g_transcript scrollRangeToVisible:NSMakeRange(ts.length, 0)];
    } else {
        [clip scrollToPoint:keep];
        [g_transcriptScroll reflectScrolledClipView:clip];
    }
}

int interpres_gui_show_menu(const InterpresMenuItem *items, int count) {
    if (!g_settings || !items || count <= 0)
        return 0;
    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Settings"];
    menu.autoenablesItems = NO;
    id target = NSApp.delegate;
    for (int i = 0; i < count; i++) {
        if (items[i].id == 0) {
            [menu addItem:[NSMenuItem separatorItem]];
            continue;
        }
        NSMenuItem *mi = [[NSMenuItem alloc] initWithTitle:str(items[i].title)
                                                    action:@selector(onMenuItem:)
                                             keyEquivalent:@""];
        mi.target = target;
        mi.tag = items[i].id;
        mi.state = items[i].checked ? NSControlStateValueOn : NSControlStateValueOff;
        mi.enabled = items[i].enabled != 0;
        [menu addItem:mi];
    }
    g_menu_choice = 0;
    NSPoint below = NSMakePoint(0, NSHeight(g_settings.bounds) + 4);
    if (!g_settings.isFlipped)
        below.y = -4;
    [menu popUpMenuPositioningItem:nil atLocation:below inView:g_settings];
    return g_menu_choice;
}

void interpres_gui_attention(int critical) {
    if (critical) {
        [NSApp requestUserAttention:NSCriticalRequest];
        NSSound *s = [NSSound soundNamed:@"Basso"];
        if (s)
            [s play];
        else
            NSBeep();
    } else {
        [NSApp requestUserAttention:NSInformationalRequest];
        NSSound *s = [NSSound soundNamed:@"Glass"];
        if (s)
            [s play];
        else
            NSBeep();
    }
}

int interpres_gui_copy_text(const char *text) {
    NSPasteboard *pb = NSPasteboard.generalPasteboard;
    [pb clearContents];
    return [pb setString:str(text) forType:NSPasteboardTypeString] ? 1 : 0;
}

int interpres_gui_pick_folder(char *buf, int buflen) {
    if (!buf || buflen < 2)
        return 0;
    buf[0] = 0;
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = NO;
    panel.canChooseDirectories = YES;
    panel.allowsMultipleSelection = NO;
    panel.canCreateDirectories = YES;
    panel.message = @"Choose where Interpres should save transcript files";
    panel.prompt = @"Use this folder";
    if ([panel runModal] != NSModalResponseOK)
        return 0;
    const char *p = panel.URLs.firstObject.path.UTF8String;
    if (!p)
        return 0;
    strncpy(buf, p, (size_t)buflen - 1);
    buf[buflen - 1] = 0;
    return 1;
}
