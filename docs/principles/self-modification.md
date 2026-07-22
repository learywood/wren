# Self-Modification

Wren must be able to modify its own behavior while it is running and apply those changes without restarting the harness process.

Self-modification must use the production extension mechanism. It must not rely on a privileged code path unavailable to extensions.
