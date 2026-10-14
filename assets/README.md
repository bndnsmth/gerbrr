`preview.png` is the README's actual GPU-rendered native workspace capture of the
built-in Atlas board, refreshed with the compact/collapsible sidebar and GitHub
header link. It contains original demo artwork, not public corpus or private
fabrication files. Regenerate it in a desktop session with:

```sh
./target/release/gerbrr screenshot assets/preview.png
```

Screenshot mode requests a fixed-size native window and exits after capture.

`gerbrr.png` is the shared native app/header icon: a gerbil in safety goggles,
holding a tiny PCB. The editable source is `web/icon.svg`; rasterize it at 256×256
to update the PNG. Both are original project artwork, covered by the MIT license.

`github.svg` is Primer Octicons' [mark-github-16](https://github.com/primer/octicons/blob/main/icons/mark-github-16.svg),
with a white fill for tinting in the shared native/browser header. Its MIT license
is included in `octicons-LICENSE.txt`; the GitHub mark remains GitHub's trademark.
The bundled `github.png` keeps icon loading local, with no new runtime dependency.
Regenerate it with:

```sh
rsvg-convert -w 64 -h 64 assets/github.svg -o assets/github.png
```
