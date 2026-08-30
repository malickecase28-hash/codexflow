# License and attribution notice

CodexFlow Native Hive in this package is newly written integration code.

Architectural inspiration was drawn from:

- OpenAI Codex, Apache-2.0: https://github.com/openai/codex
- Munder Difflin, MIT-licensed source code: https://github.com/chaitanyagiri/munder-difflin

No Munder Difflin source files, Electron UI assets, or bundled art assets are copied
into this package. The concepts adopted are control-plane patterns: a privileged
supervisor, durable task state, bounded handoffs, independent workers, and explicit
safety/budget layers.
