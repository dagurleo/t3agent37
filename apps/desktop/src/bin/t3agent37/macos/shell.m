// What AppKit gives only to Objective-C: the menu-bar item, the panel under it, notifications,
// the login item and updates. The status item follows F2's status.m; presenting the panel
// follows F2's panel.m; updates, F2's updates.m.
//
// A left click on the item calls back with T3A_TOGGLE; a right (or control) click opens its
// menu, whose items call back with their tag.

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <ServiceManagement/ServiceManagement.h>
#import <UserNotifications/UserNotifications.h>
#import <objc/message.h>

typedef void (*t3a_clicked)(int32_t what);

enum { T3A_TOGGLE = 0, T3A_OPEN = 1, T3A_QUIT = 2 };

bool t3a_updates_check(void);

static NSStatusItem *status_item;
static NSMenu *status_menu;
// The item's window at the last click: the panel opens under it.
static NSRect anchor;
static BOOL has_anchor = NO;

@interface T3AStatusTarget : NSObject <NSMenuDelegate>
@property t3a_clicked clicked;
@end

@implementation T3AStatusTarget
- (void)pressed:(id)sender {
    NSStatusBarButton *button = status_item.button;
    // (The item's window, not the button: the button is 22 pt tall inside the bar.)
    anchor = button.window.frame;
    has_anchor = YES;
    NSEvent *event = NSApp.currentEvent;
    BOOL secondary = event.type == NSEventTypeRightMouseUp || (event.modifierFlags & NSEventModifierFlagControl);
    if (secondary) {
        // Shown as the item's own menu, so it sits where menu-bar menus do.
        status_item.menu = status_menu;
        [button performClick:nil];
        return;
    }
    self.clicked(T3A_TOGGLE);
}

- (void)choose:(NSMenuItem *)item {
    self.clicked((int32_t)item.tag);
}

- (void)checkForUpdates:(id)sender {
    t3a_updates_check();
}

// Left clicks go back to the button's action once the menu is closed.
- (void)menuDidClose:(NSMenu *)menu {
    status_item.menu = nil;
}
@end

static T3AStatusTarget *status_target;

static NSMenuItem *menu_item(NSString *title, NSInteger tag) {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title action:@selector(choose:) keyEquivalent:@""];
    item.target = status_target;
    item.tag = tag;
    return item;
}

// `png1x`/`png2x`: the 22 pt template icon at 1× and 2×.
void t3a_status_install(const uint8_t *png1x, size_t len1x, const uint8_t *png2x, size_t len2x, const char *tooltip,
                        t3a_clicked clicked) {
    @autoreleasepool {
        status_target = [T3AStatusTarget new];
        status_target.clicked = clicked;

        NSImage *icon = [[NSImage alloc] initWithSize:NSMakeSize(22, 22)];
        for (NSData *data in @[ [NSData dataWithBytes:png1x length:len1x], [NSData dataWithBytes:png2x length:len2x] ]) {
            NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:data];
            rep.size = NSMakeSize(22, 22);
            [icon addRepresentation:rep];
        }
        icon.template = YES;

        status_menu = [NSMenu new];
        status_menu.autoenablesItems = NO;
        status_menu.delegate = status_target;
        [status_menu addItem:menu_item(@"Open t3agent37", T3A_OPEN)];
        [status_menu addItem:NSMenuItem.separatorItem];
        [status_menu addItem:menu_item(@"Quit t3agent37", T3A_QUIT)];

        status_item = [NSStatusBar.systemStatusBar statusItemWithLength:NSVariableStatusItemLength];
        NSStatusBarButton *button = status_item.button;
        button.image = icon;
        button.toolTip = @(tooltip);
        button.accessibilityLabel = @"t3agent37";
        button.target = status_target;
        button.action = @selector(pressed:);
        [button sendActionOn:NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp];
    }
}

void t3a_status_tooltip(const char *tooltip) {
    status_item.button.toolTip = @(tooltip);
}

// The panel GPUI made hidden: no title bar at all (GPUI's title-less window keeps a hidden
// one), `width`×`height` pt, its top `gap` pt under the item and centred on it, kept on the
// item's screen; drawn now, then put in front and given the keyboard (F2's take_focus: an
// accessory app has to activate itself for its panel to be key).
//
// On the next turn of the main loop: called from inside GPUI, and AppKit calls GPUI right
// back (resize, key, draw), which it can't take while it's busy.
static void present(NSView *view, double width, double height, double gap);

void t3a_panel_present(void *view_ptr, double width, double height, double gap) {
    NSView *view = (__bridge NSView *)view_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
      present(view, width, height, gap);
    });
}

static void present(NSView *view, double width, double height, double gap) {
    @autoreleasepool {
        NSWindow *window = view.window;
        window.styleMask = NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel | NSWindowStyleMaskFullSizeContentView;
        NSScreen *screen = status_item.button.window.screen ?: NSScreen.mainScreen;
        NSRect visible = screen.visibleFrame;
        // A big panel never runs past the bottom of the screen.
        height = MIN(height, NSHeight(visible) - gap - 8);
        CGFloat x = has_anchor ? NSMidX(anchor) - width / 2 : NSMaxX(visible) - width - 8;
        x = MAX(NSMinX(visible) + 6, MIN(x, NSMaxX(visible) - width - 6));
        CGFloat top = (has_anchor ? NSMinY(anchor) : NSMaxY(visible)) - gap;
        [window setFrame:NSMakeRect(x, top - height, width, height) display:NO];
        view.frame = window.contentView.bounds;
        [window makeFirstResponder:view];
        window.movableByWindowBackground = NO;
        // Rounded as the system rounds windows; the shadow follows the clipped shape.
        window.opaque = NO;
        window.backgroundColor = NSColor.clearColor;
        view.wantsLayer = YES;
        view.layer.cornerRadius = 10;
        view.layer.cornerCurve = kCACornerCurveContinuous;
        view.layer.masksToBounds = YES;
        window.hasShadow = YES;

        [view.layer setNeedsDisplay];
        [view.layer displayIfNeeded];
        [CATransaction flush];
        [NSRunningApplication.currentApplication activateWithOptions:0];
        [window makeKeyAndOrderFront:nil];
        [window invalidateShadow];
        if (getenv("T3A_DEBUG")) {
            NSLog(@"t3a present: screen %@ (scale %.1f) visible %@ anchor %@ (%d) got %@", NSStringFromRect(screen.frame),
                  screen.backingScaleFactor, NSStringFromRect(visible), NSStringFromRect(anchor), has_anchor,
                  NSStringFromRect(window.frame));
        }
    }
}

// Hidden, not closed: it keeps everything it shows for the next click. (Next turn, as above.)
void t3a_panel_hide(void *view_ptr) {
    NSView *view = (__bridge NSView *)view_ptr;
    dispatch_async(dispatch_get_main_queue(), ^{
      [view.window orderOut:nil];
    });
}

// ── app bundle features ──────────────────────────────────────────────

// Notifications and the login item belong to an app bundle; `cargo run` has none.
bool t3a_bundled(void) {
    return NSBundle.mainBundle.bundleIdentifier != nil && [NSBundle.mainBundle.bundlePath hasSuffix:@".app"];
}

void t3a_notify(const char *title, const char *body) {
    NSString *t = @(title), *b = @(body);
    UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
    [center requestAuthorizationWithOptions:UNAuthorizationOptionAlert | UNAuthorizationOptionSound
                          completionHandler:^(BOOL granted, __unused NSError *error) {
                            if (!granted) return;
                            UNMutableNotificationContent *content = [UNMutableNotificationContent new];
                            content.title = t;
                            content.body = b;
                            NSString *id = NSUUID.UUID.UUIDString;
                            [center addNotificationRequest:[UNNotificationRequest requestWithIdentifier:id
                                                                                                content:content
                                                                                                trigger:nil]
                                     withCompletionHandler:nil];
                          }];
}

bool t3a_login_item_on(void) {
    return SMAppService.mainAppService.status == SMAppServiceStatusEnabled;
}

// 0 when done; else the error is logged and 1 returned.
int t3a_login_item_set(bool on) {
    NSError *error = nil;
    SMAppService *app = SMAppService.mainAppService;
    if (on == (app.status == SMAppServiceStatusEnabled)) return 0;
    BOOL done = on ? [app registerAndReturnError:&error] : [app unregisterAndReturnError:&error];
    if (!done) NSLog(@"t3agent37 login item: %@", error);
    return done ? 0 : 1;
}

// Is Reduce motion on? The rooftop then holds still.
bool t3a_reduce_motion(void) {
    return NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion;
}

// Updates: Sparkle, loaded from the app's Frameworks when the build has it (a bundle made by
// scripts/bundle-macos.sh does; `cargo run` doesn't) and Info.plist says where updates are and
// what they're signed with. Its own window and checks, once a day. Whether it's on. Puts "Check
// for Updates…" in the menu-bar item's menu, so call it after t3a_status_install.
static id updater;

bool t3a_updates_start(void) {
    @autoreleasepool {
        NSDictionary *info = NSBundle.mainBundle.infoDictionary;
        if ([info[@"SUFeedURL"] length] == 0 || [info[@"SUPublicEDKey"] length] == 0) {
            return false;
        }
        NSString *path = [NSBundle.mainBundle.privateFrameworksPath stringByAppendingPathComponent:@"Sparkle.framework"];
        NSBundle *sparkle = [NSBundle bundleWithPath:path];
        NSError *error = nil;
        if (!sparkle || ![sparkle loadAndReturnError:&error]) {
            NSLog(@"Sparkle didn't load: %@", error);
            return false;
        }
        Class controller = NSClassFromString(@"SPUStandardUpdaterController");
        if (!controller) {
            return false;
        }
        SEL make = NSSelectorFromString(@"initWithStartingUpdater:updaterDelegate:userDriverDelegate:");
        updater = ((id(*)(id, SEL, BOOL, id, id))objc_msgSend)([controller alloc], make, YES, nil, nil);
        if (updater && status_menu) {
            NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"Check for Updates…" action:@selector(checkForUpdates:) keyEquivalent:@""];
            item.target = status_target;
            [status_menu insertItem:item atIndex:1];
        }
        return updater != nil;
    }
}

// Look for an update now, with Sparkle's window in front. False if updates are off. (On the
// next turn of the main loop: it's called from inside GPUI too.)
bool t3a_updates_check(void) {
    if (!updater) {
        return false;
    }
    dispatch_async(dispatch_get_main_queue(), ^{
      [NSApp activateIgnoringOtherApps:YES];
      ((void (*)(id, SEL, id))objc_msgSend)(updater, NSSelectorFromString(@"checkForUpdates:"), nil);
    });
    return true;
}
