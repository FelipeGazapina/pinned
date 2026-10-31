# Executable plugin hooks

A hook script that is executable on the machine that publishes a plugin stays executable on the other machines. Claude Code skips a hook it cannot execute, so a remote session would otherwise load the plugin and never run it.

`executable` on a file entry is optional. A unix machine sends true or false after it reads the mode. Windows and older apps omit the field. Omitted means the sender does not know the mode.

The hub stores the bit for files under `pinned:plugins`. A push that omits the field leaves the stored bit as it is. A receiver sets the owner execute bit when the entry says true, and clears execute bits only when the entry says false. When the field is missing, the receiver keeps the mode the file already has.

The same bytes keep the executable bit. A newer plain copy does not clear an older executable one. The bit clears only when a unix machine changes the bytes and reports that the file is not executable. After the machines and the hub agree, a later sync sends nothing.

A machine that already has those bytes, and can execute them while the hub cannot, does not send the file again. It sends `sync_mark_executable` with the path and the sha it matched. The hub sets the bit only while the stored sha is still that sha. It does not change the bytes or the modified time. A repair that arrives after another machine has stored new bytes changes nothing.

A machine that already has those bytes, and cannot execute them while the hub can, changes the mode of the file it has. It does not download the file or write it again.

An old hub omits `executable`. A new client then sends no repair. An old client still sends a full file push when it wants to repair the mode, and a new hub stores that push as a content write. A new client and a new hub use the repair call.

`chmod` on the install reaches the other machines even when it does not change the modification time. The mirror copies that mode change. The receiver adds owner execute only. It does not add group or other execute.

Sessions, memory, project skills, and history do not carry the bit. They are not hook scripts. Windows does not read or apply the bit.

Names the sync already refuses stay refused. That includes `settings.json`, `settings.local.json`, `.claude.json`, `installed_plugins.json`, names that start with `.env`, credentials, and keys.
