# notes — a worked plugin

The smallest plugin that uses everything a plugin has: a database of its own, a builtin, and a
row-producing tool.

```sh
oslo plugin install examples/plugins/notes
note "the shell is the plugin host"
note                       # every note, one per line
notes | where 'note:match("shell")' | cols at
```

Its data lives in `$XDG_DATA_HOME/oslo/plugins/notes.kv`, mode `0600`, and `oslo plugin remove
notes` leaves it there.
