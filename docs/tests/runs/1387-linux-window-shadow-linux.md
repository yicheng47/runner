# #1387 Linux window shadow

Tested on Arch Linux in a KDE Plasma Wayland session at 2× display scale, with the development build from `09c259b8` plus the shadow change. The selected live check covers the floating window's outer shadow on a light background.

| Check | Result |
| --- | --- |
| Original left edge | The outermost shadow pixel in the supplied desktop capture was RGB (239, 239, 239), next to background RGB (252, 252, 252). The difference makes a straight line visible. |
| Updated floating window | Captured the 2880 × 1800 window as RGBA with Spectacle. Sampled alpha inward from each edge at 20%, 50% and 80% along its length. All 12 profiles increased monotonically towards the frame. The outermost 5 pixels on each side, 7 at the top and 3 at the bottom were fully transparent. |
| Resize margin | All 12 profiles reached the opaque border after 12 physical pixels (6 logical pixels). All eight `platform_ui::linux::tests` passed, including resize hit testing along four sides and four corners. |

The [visual issue](https://github.com/yicheng47/runner/issues/1387) includes the before and after detail. Live verification covered the floating window on this compositor and scale.
