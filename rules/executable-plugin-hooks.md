# Executable plugin hooks

A hook script that is executable on the machine that publishes a plugin stays executable on the other machines. Claude Code skips a hook it cannot execute, so a remote session would otherwise load the plugin and never run it.

The file entry carries `executable`. An older app omits the field, and the file is written as a normal file. A receiver that already has the same bytes without the bit becomes executable on the next sync. When the modification times match, the executable copy wins, so the two machines do not trade the bit. A newer file's mode wins, as a newer file's bytes do.

Windows does not read or apply the bit.

Names the sync already refuses stay refused. That includes `settings.json`, `settings.local.json`, `.claude.json`, `installed_plugins.json`, names that start with `.env`, credentials, and keys.
