# Runner shell integration for zsh. Runner points ZDOTDIR here for the one
# file zsh reads first, so put the user's ZDOTDIR back before anything else:
# the rest of their startup files then load from where they always do.
if [[ -n "${RUNNER_ZSH_ZDOTDIR+X}" ]]; then
  builtin export ZDOTDIR="$RUNNER_ZSH_ZDOTDIR"
  builtin unset RUNNER_ZSH_ZDOTDIR
else
  builtin unset ZDOTDIR
fi

{
  builtin typeset _runner_file="${ZDOTDIR-$HOME}/.zshenv"
  [[ ! -r "$_runner_file" ]] || builtin source -- "$_runner_file"
} always {
  if [[ -o interactive ]]; then
    # Whether the words in $1 (hook names, prompt strings), or a function
    # reachable from them, send OSC 7. Functions already found clean are in
    # __runner_osc7_seen and are skipped. Bounded to 256 functions and
    # 512 KiB of source per call.
    __runner_osc7_sent_elsewhere() {
      builtin emulate -L zsh
      builtin local -a names queue next
      builtin local f body budget=256 bytes=524288
      builtin local IFS=$' \t\n$(){};|&"\'`<>=[]!'
      [[ $1 == *']7;'* ]] && return 0
      names=(${(k)functions})
      queue=(${=1})
      while (( $#queue && budget > 0 && bytes > 0 )); do
        next=()
        for f in ${(u)${queue:*names}}; do
          [[ $f == __runner_osc7* || -n ${__runner_osc7_seen[$f]-} ]] && continue
          __runner_osc7_seen[$f]=1
          body=${functions[$f]-}
          if [[ $body == 'builtin autoload -X'* ]]; then
            builtin autoload +X -- $f 2>/dev/null
            body=${functions[$f]-}
          fi
          [[ $body == *']7;'* ]] && return 0
          (( budget -= 1, bytes -= $#body ))
          next+=(${=body})
        done
        queue=($next)
      done
      return 1
    }
    # OSC 7 on each prompt. When the prompt hooks change, check them all
    # again; when only the prompt strings change, check those. If the user's
    # configuration turns out to send OSC 7 too, remove this hook.
    __runner_osc7() {
      builtin emulate -L zsh
      builtin setopt no_multibyte
      builtin local hooks="${(j: :)precmd_functions}|${(j: :)chpwd_functions}"
      builtin local prompts="${PS1-} ${RPS1-}" roots=
      if [[ $hooks != "${__runner_osc7_hooks-}" ]]; then
        typeset -gA __runner_osc7_seen=()
        roots="precmd chpwd $hooks $prompts"
      elif [[ $prompts != "${__runner_osc7_prompts-}" ]]; then
        roots=$prompts
      fi
      typeset -g __runner_osc7_hooks=$hooks __runner_osc7_prompts=$prompts
      if [[ -n $roots ]] && __runner_osc7_sent_elsewhere "$roots"; then
        precmd_functions=(${precmd_functions:#__runner_osc7})
        return 0
      fi
      builtin local dir="$PWD"
      if [[ $dir == *[^A-Za-z0-9/._~-]* ]]; then
        builtin local c hex encoded=
        for c in ${(s::)dir}; do
          if [[ $c == [A-Za-z0-9/._~-] ]]; then
            encoded+=$c
          else
            builtin printf -v hex '%%%02X' "'$c"
            encoded+=$hex
          fi
        done
        dir=$encoded
      fi
      # No host: this hook only runs in a local shell, and $HOST, read once at
      # startup, goes stale when a laptop's hostname follows the network.
      builtin printf '\e]7;file://%s\e\\' "$dir"
    }
    (( ${precmd_functions[(Ie)__runner_osc7]} )) || precmd_functions+=(__runner_osc7)
  fi
  builtin unset _runner_file
}
