# Executable plugin hooks

A hook script that is executable on the machine that publishes a plugin stays executable on the other machines. Claude Code skips a hook it cannot execute, so a remote session would otherwise load the plugin and never run it.

`executable` on a file entry is optional. A unix machine sends true or false after it reads the mode. Windows and older apps omit the field. Omitted means the sender does not know the mode.

The hub stores the bit for files under `pinned:plugins`. A push that omits the field leaves the stored bit as it is. A receiver sets the owner execute bit when the entry says true, and clears execute bits only when the entry says false. When the field is missing, the receiver keeps the mode the file already has.

The same bytes keep the executable bit. A newer plain copy does not clear an older executable one. The bit clears only when a unix machine changes the bytes and reports that the file is not executable. After the machines and the hub agree, a later sync sends nothing.

`chmod` on the install reaches the other machines even when it does not change the modification time. The mirror copies that mode change. The receiver adds owner execute only. It does not add group or other execute.

Sessions, memory, project skills, and history do not carry the bit. They are not hook scripts. Windows does not read or apply the bit.

Names the sync already refuses stay refused. That includes `settings.json`, `settings.local.json`, `.claude.json`, `installed_plugins.json`, names that start with `.env`, credentials, and keys.
