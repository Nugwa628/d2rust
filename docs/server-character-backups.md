# Server character backups

M6a provides offline backups of M3's server character store. Accounts and
login, automatic backup scheduling, retention, and live snapshots remain open.

Stop the server and any other process writing the configured character folder.
Run:

```text
d2rust-server --config /srv/d2/server.cfg --backup /srv/backups/before-update
```

The backup destination must be new, its parent must exist, and it must be
outside the character folder. Relative backup paths use the command's working
directory; paths in the config still use the config's directory. The configured
character folder must already exist. Backup mode does not read game data,
open the log, start a game, or connect to a network.

Only named `.d2rc` characters are copied. Each envelope is validated, including
its version and checksum, and copied without conversion. Unfinished `.tmp`
saves and unrelated files are excluded. Non-file character entries and names
that collide without regard to case are refused.

A successful command exits zero and reports the count. `BACKUP-COMPLETE` is
published after every copied file is flushed. Any failure exits nonzero; a
partial directory may remain for diagnosis, without that completion marker.
Never restore an incomplete backup or reuse its destination. Keep the previous
completed backup until a new backup succeeds. File flushing does not promise
directory-entry durability through power loss on every filesystem; keep
independent backups for that risk.

To restore, keep all character writers stopped, verify `BACKUP-COMPLETE` exists
in a successful backup, and copy its `.d2rc` files into a **new, empty** character
folder. Point the server config at that folder and restart. Do not merge old
records into an active store. Keep both the previous live folder and backup
until the restored characters have been checked. Online characters remain on
the server; this command never imports local single-player saves.
