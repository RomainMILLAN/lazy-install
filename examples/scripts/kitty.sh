#!/usr/bin/env bash
# lazy-install: v1
#
# kitty, installed in ~/.local/kitty.app by its official installer.
# The latest version is read from the redirect of /releases/latest, which does
# not count against the GitHub API rate limit.

needs_update() {
  local installed latest
  installed="$("$HOME/.local/kitty.app/bin/kitty" --version 2>/dev/null | awk '{print $2}')"
  latest="$(curl -fsSI https://github.com/kovidgoyal/kitty/releases/latest \
    | sed -n 's#^location:.*/tag/v\([^[:space:]]*\).*#\1#ip' | tr -d '\r')"
  [[ -n $latest ]] || return 2   # no helper called: shows as "error"
  if [[ $installed == "$latest" ]]; then
    li_up_to_date "$installed" "$latest"
  else
    li_update_available "$installed" "$latest"
  fi
}

update() {
  curl -fsSL https://sw.kovidgoyal.net/kitty/installer.sh | sh /dev/stdin launch=n
}
