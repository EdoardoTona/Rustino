# TODO

## Global shortcuts on Wayland

Global shortcuts use the `global-hotkey` crate, which grabs keys through X11. Wayland doesn't let apps grab keys:

- In a Wayland session with XWayland (e.g. GNOME on Ubuntu) a shortcut registers, but it only fires while an X11
  window has the focus.
- Without an X11 display `RegisterGlobalShortcut` returns `false`.

The native way is the `org.freedesktop.portal.GlobalShortcuts` portal of xdg-desktop-portal (D-Bus):

- The app proposes shortcuts with a description; the desktop asks the user to confirm them and may change their keys.
- Presses arrive as the `Activated` / `Deactivated` signals of the session.
- It needs an app ID (with its `.desktop` file).
- Supported by KDE Plasma 5.27+, Hyprland and GNOME 48+ (not GNOME 46 on Ubuntu 24.04; Ubuntu 25.04+ has it).

Plan: a Linux backend used when there's no X11 display, through gio's D-Bus (already a dependency through GTK).

- Registration becomes asynchronous and depends on a system dialog: `RegisterGlobalShortcut` can't answer
  `true`/`false` right away. Decide how to report the outcome (e.g. an event, or a `Task<bool>`).
- Map the accelerator syntax to the portal's `preferred_trigger` format.
- Test on a desktop with the portal (KDE, or a newer Ubuntu than the Proxmox VM's 24.04).
