Yettel internet settings
=======================
Get the PPPoE username and password for your own Yettel / Cetin Serbia line.
Unofficial; not affiliated with Yettel, Cetin or ZTE.
Use only your own router and subscription.

Before connecting
-----------------
The app identifies itself to the provider as your router. The provider may
send new management credentials or make changes that cancellation cannot undo.
Credentials are stored on this computer. The app does not configure your
router or your computer's network settings. A warning appears before every
connection.

Run
---
Extract all files first. No installation or administrator rights are needed.
Windows: double-click yettel-cwmp.exe or run it in Windows Terminal.
macOS/Linux: open Terminal in this folder, then:
  chmod +x ./yettel-cwmp
  ./yettel-cwmp
Downloads are unsigned. Verify their origin and SHA256SUMS before opening.
Follow the full guide below for operating-system security warnings.
Use a terminal at least 52 columns by 16 rows.
--lang en or --lang sr selects English or Serbian Latin.
NO_COLOR=1 disables colors. --help and --version work without a terminal.

Choose Add a router (N). Enter the serial number, MAC address and case-sensitive
Wi-Fi key from your ZTE H3600P label. Ctrl+R or F2 reveals/hides the key.
Choose Receive internet settings (R), read the warning, then continue.
S reveals/hides credentials; C copies the password; U copies the username.
PgUp/PgDn and Home/End scroll. V opens saved settings offline.
K changes the key; replacing provider-issued credentials needs confirmation.
D deletes a router's local data after confirmation.
Esc goes back/cancels. Q quits menus; Ctrl+C quits from anywhere. ?/F1 opens help.
Receiving credentials does not configure a replacement router or TV service.
VLAN and MTU values are reference presets; confirm them for your line.

Your data
---------
Saved files contain plaintext credentials. Keep them in a private local folder.
Linux:   ~/.local/state/yettel-cwmp (or $XDG_STATE_HOME/yettel-cwmp)
Windows: %LOCALAPPDATA%\yettel-cwmp
macOS:   ~/Library/Application Support/yettel-cwmp
--state-dir DIR chooses another folder. Delete a router inside the app,
or close the app before removing its data folder. Deletion cannot undo
provider changes or erase copies in backups and clipboard history.
Never include label values, keys, passwords or saved files in an issue.

Full guide: https://github.com/stlk0/yettel-cwmp#readme
Downloads:  https://github.com/stlk0/yettel-cwmp/releases/latest
Security:   https://github.com/stlk0/yettel-cwmp/security/policy

License: MIT; see LICENSE and THIRD_PARTY_LICENSES.html.
