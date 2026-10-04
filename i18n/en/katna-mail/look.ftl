# Katna Mail, English: Scaling and Look & Feel.
# Guide: i18n/README.md. Keep ids stable; change the text freely.
# In plural forms, write { $count } rather than the digit, so languages
# with their own digits show them.

## Settings > Appearance > Scaling

# A sample letter drawn small and large at the two ends of the scale slider.
# Use a common letter of your script.
scale-letter = A
# The interface scale. $percent: a number such as 125.
scale-percent = { $percent }%
# A button that sets the scale back to normal. $percent is 100.
scale-reset = Back to { $percent }%

## Settings > Experimental > Look & Feel

look-intro = Features still being tried out. They may change or go away.
look-heading = Look & Feel
# A row's name; the Settings search finds it by this name too.
look-window-frame = Window frame
look-window-frame-detail = Who draws the title bar, the window buttons, the corners and the shadow.
# The desktop draws the window's frame. KDE and Plasma are names.
look-frame-native-kde = Native: KDE's frame, in your Plasma theme
look-frame-native = Native: the desktop's frame
# On Windows.
look-frame-native-windows = Native: Windows' own title bar
look-frame-katna = Katna: the top bar becomes the title bar
# Under the Katna frame choice. $desktop: the desktop's name, such as KDE or GNOME.
look-frame-katna-note-named = Katna draws rounded corners and its own shadow. The frame no longer follows the { $desktop } theme; window rules still apply.
# As look-frame-katna-note-named, when the desktop's name is not known.
look-frame-katna-note = Katna draws rounded corners and its own shadow. The frame no longer follows the desktop theme; window rules still apply.
# As look-frame-katna-note, on Windows.
look-frame-katna-note-windows = Katna's top bar becomes the title bar. Windows still rounds the corners and draws the shadow.
# A slider under the Katna frame choice: how round the window's corners are.
look-window-radius = Corner roundness
look-window-radius-square = Square
look-window-radius-round = Round
# The unit after the roundness typed beside its slider: pixels.
look-px = px
# A switch under the Katna frame choice.
look-window-border = Border
look-window-border-detail = A thin line around the window
# A slider under the Border switch.
look-window-border-opacity = Border opacity
look-window-border-faint = Faint
look-window-border-strong = Strong
# On Windows, after the frame choice changed: the open window keeps its frame.
look-frame-on-reopen = The frame changes the next time you open Katna Mail.
# Shown in place of the frame choices on a desktop where every app draws its own frame.
look-frame-client-side = Your desktop leaves the frame to each app, so Katna already draws its own.
look-blurred-background = Blur
# Under "Blurred background".
look-blurred-background-detail = Choose where Katna Mail uses blur. Turn on either one, or both.
look-blur = Blur the window background
look-blur-detail = The desktop shows through the top bar and the folders
look-frosted-panes = Frosted panes
look-frosted-panes-detail = The mail list, the open mail and the person card let the blur through too
look-pane-opacity = Pane opacity
look-frosted-chat = Frosted chat background
look-frosted-chat-detail = Behind a chat's bubbles the blur shows a little more; the bubbles stay solid
look-frosted-search = Frosted search box
look-frosted-search-detail = The search box lets the blur through while you type in it
look-frosted-headers = Frosted headers
look-frosted-headers-detail = The bars at the top of the mail list, the open mail and a chat blur what scrolls under them
look-frosted-popups = Frosted menus and dialogs
look-frosted-popups-detail = Menus, popovers, dialogs and viewer bars blur what is under them
look-custom-frost = Custom blur amount
look-custom-frost-on = Blur strength sets menus and dialogs. Opacity sets them and the blurred window background.
look-custom-frost-off-kde = Off: KDE's blur strength and Katna's opacity
look-custom-frost-off = Off: Katna's blur and opacity
look-frost-blur = Blur strength: menus and dialogs
look-frost-blur-light = Light
look-frost-blur-strong = Strong
look-frost-opacity = Opacity: menus, dialogs and the window background
look-frost-opacity-clear = See-through
look-frost-opacity-solid = Solid
look-kde-blur-note = The window background's blur strength always comes from KDE.
look-kde-blur-open = Open KDE's Blur settings
# Why frosted menus are not available.
look-frosted-popups-none-windows = Katna Mail cannot frost menus on Windows yet.
look-frosted-popups-none = Your graphics driver does not let Katna Mail blur under menus.
# Why blur is not available. "Blur", "System Settings", "Window Management" and
# "Desktop Effects" are KDE's own names for its settings; use KDE's translation of them.
look-blur-off-kde = KDE's blur effect is off. Turn on Blur in System Settings, Window Management, Desktop Effects, then open Katna Mail again.
look-blur-none-gnome = GNOME does not blur what is behind windows.
look-blur-none-x11 = Your window manager does not blur what is behind windows.
look-blur-none-wayland = Your compositor does not blur what is behind windows.
