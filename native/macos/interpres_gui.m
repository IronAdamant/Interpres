/*
 * Interpres native macOS UI — AppKit only (system frameworks).
 * Large high-contrast controls for Deaf / hard-of-hearing users.
 */
#import <Cocoa/Cocoa.h>
#include "interpres_gui.h"
#include <string.h>

static InterpresGuiCallbacks g_cbs;
static NSTextField *g_status;
static NSTextView *g_live;
static NSTextView *g_history;
static NSTextField *g_folder;
static NSTextField *g_session;
static NSButton *g_startBtn;
static NSButton *g_stopBtn;
static NSButton *g_rememberBtn;
static NSButton *g_debugBtn;
static NSWindow *g_window;

static NSColor *bgColor(void) {
    return [NSColor colorWithCalibratedRed:0.07 green:0.08 blue:0.10 alpha:1.0];
}
static NSColor *panelColor(void) {
    return [NSColor colorWithCalibratedRed:0.12 green:0.13 blue:0.16 alpha:1.0];
}
static NSColor *accentColor(void) {
    return [NSColor colorWithCalibratedRed:0.20 green:0.75 blue:0.55 alpha:1.0];
}
static NSColor *textColor(void) {
    return [NSColor colorWithCalibratedWhite:0.95 alpha:1.0];
}
static NSColor *mutedColor(void) {
    return [NSColor colorWithCalibratedWhite:0.65 alpha:1.0];
}

static NSButton *makeButton(NSString *title, id target, SEL action, NSRect frame) {
    NSButton *b = [[NSButton alloc] initWithFrame:frame];
    [b setTitle:title];
    [b setBezelStyle:NSBezelStyleRegularSquare];
    [b setFont:[NSFont boldSystemFontOfSize:18.0]];
    [b setTarget:target];
    [b setAction:action];
    [b setWantsLayer:YES];
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
    [content addSubview:makeLabel(@"Interpres", NSMakeRect(24, h - 56, 400, 36), 28, YES)];
    [content addSubview:makeLabel(@"Saves what Live Captions say — on this Mac only",
                                   NSMakeRect(24, h - 84, 600, 24), 14, NO)];

    /* Big controls row */
    g_startBtn = makeButton(@"▶  Start listening", self, @selector(onStart:),
                            NSMakeRect(24, h - 150, 220, 52));
    g_stopBtn = makeButton(@"■  Stop", self, @selector(onStop:),
                           NSMakeRect(256, h - 150, 140, 52));
    [g_stopBtn setEnabled:NO];
    g_rememberBtn = makeButton(@"Save to disk: OFF", self, @selector(onRemember:),
                               NSMakeRect(412, h - 150, 220, 52));
    NSButton *folderBtn = makeButton(@"Choose folder…", self, @selector(onChooseFolder:),
                                     NSMakeRect(648, h - 150, 180, 52));
    NSButton *openBtn = makeButton(@"Open folder", self, @selector(onOpenFolder:),
                                   NSMakeRect(648, h - 210, 180, 44));
    NSButton *checkBtn = makeButton(@"Check setup", self, @selector(onCheck:),
                                    NSMakeRect(24, h - 210, 180, 44));
    g_debugBtn = makeButton(@"Debug: OFF", self, @selector(onDebug:),
                            NSMakeRect(220, h - 210, 160, 44));

    [content addSubview:g_startBtn];
    [content addSubview:g_stopBtn];
    [content addSubview:g_rememberBtn];
    [content addSubview:folderBtn];
    [content addSubview:openBtn];
    [content addSubview:checkBtn];
    [content addSubview:g_debugBtn];

    /* Status */
    [content addSubview:makeLabel(@"Status", NSMakeRect(24, h - 250, 100, 20), 13, YES)];
    g_status = makeLabel(@"Turn on Live Captions, then press Start listening.",
                         NSMakeRect(24, h - 286, w - 48, 36), 16, NO);
    [g_status setTextColor:mutedColor()];
    [content addSubview:g_status];

    /* Live line */
    [content addSubview:makeLabel(@"Live captions", NSMakeRect(24, h - 320, 200, 20), 13, YES)];
    NSTextView *liveLocal = nil;
    NSScrollView *liveScroll =
        makeScrollText(NSMakeRect(24, h - 430, w - 48, 100), &liveLocal, 22.0);
    g_live = liveLocal;
    [content addSubview:liveScroll];

    /* History */
    [content addSubview:makeLabel(@"This session", NSMakeRect(24, h - 460, 200, 20), 13, YES)];
    NSTextView *histLocal = nil;
    NSScrollView *histScroll =
        makeScrollText(NSMakeRect(24, 70, w - 48, h - 540), &histLocal, 16.0);
    g_history = histLocal;
    [histScroll setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
    [content addSubview:histScroll];
    /* Folder + session path */
    g_folder = makeLabel(@"Folder: (not set)", NSMakeRect(24, 36, w - 48, 22), 13, NO);
    [g_folder setTextColor:mutedColor()];
    [content addSubview:g_folder];
    g_session = makeLabel(@"", NSMakeRect(24, 12, w - 48, 22), 12, NO);
    [g_session setTextColor:mutedColor()];
    [content addSubview:g_session];

    [g_window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
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
    });
}

void interpres_gui_set_listening(int on) {
    on_main(^{
      if (g_startBtn)
          [g_startBtn setEnabled:!on];
      if (g_stopBtn)
          [g_stopBtn setEnabled:on];
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
