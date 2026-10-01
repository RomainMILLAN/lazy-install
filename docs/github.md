# GitHub repository settings

Applied when the repository is created (`gh` cannot set the social preview).

```sh
gh repo edit RomainMILLAN/lazy-install \
  --description "Know which of your apps need an update, and run the update in an embedded terminal. A TUI in Rust." \
  --add-topic tui --add-topic rust --add-topic ratatui --add-topic terminal \
  --add-topic updater --add-topic dotfiles --add-topic package-manager --add-topic cli
```

- **Social preview**: Settings → General → Social preview → upload
  `docs/assets/social-preview.png` (2560×1280, 2:1 as GitHub expects).
- **Avatar / app icon**: `docs/assets/icon-512.png`.
- **Accent**: violet — `#A78BFA` on dark, `#6D28D9` on light (badges use
  `#6D28D9`). Each lazy-* has its own accent; lazy-transfer is teal.
- Regenerate every image: `cargo run --example screenshots && bash docs/assets/src/render.sh`.

## Claude Design

The brand lives in the Claude Design project **lazy-install — Brand**
(`https://claude.ai/design/p/d1fd39b5-bd95-458b-a821-8af1c875fdd4`): marks,
icon, banners, social preview, screens, `tokens/colors.css` and a brand guide.
Changes made there come back here as files in `docs/assets/` (and the palette
in `src/ui/style/theme.rs`, which stays the source of truth for the app).
