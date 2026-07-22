# Extension

Every Wren feature must be implemented as an extension.

This applies at every scale: basic tools such as edit and find are extensions; providers are extensions; MCP support is an extension. Features must use the same extension mechanism rather than receiving privileged, feature-specific integration in the harness.
