# Screen Sharing

A COSMIC applet for sharing your screen to wired displays (HDMI / VGA /
DisplayPort) and wireless receivers (FCast) from a single place in the panel.

> **Status:** project scaffold — sharing targets and receivers will be
> implemented in upcoming milestones.

## Building

```sh
just build-release
sudo just install
```

Restart the panel and add the applet:

```sh
pkill cosmic-panel
```

Then enable **Screen Sharing** in **Settings → Desktop → Panel → Applets**.

## Development

```sh
just check          # Type-check (cargo check)
just lint           # Clippy (same flags as CI)
just run            # Run standalone for testing
just flatpak-install  # Build & install a local Flatpak test build
```

## License

[MPL-2.0](LICENSE)
