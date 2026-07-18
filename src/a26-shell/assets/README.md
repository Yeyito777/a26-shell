# System app icon

`system-app.png` was generated for the A26 shell with the project image tool.
It intentionally uses the existing navy, blue, mint and white palette and has
no text. `scripts/a26-shell/prepare-assets.py` deterministically resizes it to
220x220 and emits Xorg-native BGRX bytes in `system-app.bgrx`.

The shell draws the icon's visible frame itself at exactly one physical pixel;
the generated image does not contain a surrounding UI border.
