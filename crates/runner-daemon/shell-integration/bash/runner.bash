# Runner shell integration for bash. Runner starts bash with PROMPT_COMMAND
# set to source this file, so it runs once, at the first prompt, after bash
# has read the user's startup files itself. It swaps that bootstrap for the
# hook, stops exporting PROMPT_COMMAND, and reports the first directory.
__runner_osc7_status=$?

# Whether the words in $1 (hook names, prompt strings), or a function
# reachable from them, send OSC 7. Functions already found clean are listed
# in __runner_osc7_seen and are skipped. Bounded to 64 functions and 20000
# words per call.
__runner_osc7_sent_elsewhere() {
  builtin local IFS=$' \t\n$(){};|&"\'`<>=[]!' f body i=0 budget=64
  builtin local -a queue words
  [[ $1 == *']7;'* ]] && return 0
  builtin read -r -d '' -a queue <<< "$1"
  while (( i < ${#queue[@]} && i < 20000 && budget > 0 )); do
    f=${queue[i]}
    i=$((i + 1))
    [[ -z $f || $f == __runner_osc7* ]] && continue
    builtin declare -F -- "$f" >/dev/null 2>&1 || continue
    [[ $__runner_osc7_seen == *" $f "* ]] && continue
    __runner_osc7_seen+="$f "
    budget=$((budget - 1))
    body=$(builtin declare -f -- "$f")
    [[ $body == *']7;'* ]] && return 0
    builtin read -r -d '' -a words <<< "$body"
    queue+=("${words[@]}")
  done
  return 1
}

# Drops `__runner_osc7` from one PROMPT_COMMAND entry with the separator
# beside it, leaving the result in __runner_osc7_entry.
__runner_osc7_drop() {
  builtin local nl=$'\n' pattern
  __runner_osc7_entry=$1
  if [[ $1 == __runner_osc7 ]]; then
    __runner_osc7_entry=
    return 0
  fi
  for pattern in "__runner_osc7${nl}" "${nl}__runner_osc7" "__runner_osc7; " "; __runner_osc7" "__runner_osc7;" ";__runner_osc7"; do
    if [[ $1 == *"$pattern"* ]]; then
      __runner_osc7_entry=${1/"$pattern"/}
      return 0
    fi
  done
  __runner_osc7_entry=${1/__runner_osc7/:}
}

# OSC 7 on each prompt. When the prompt hooks change, check them all again;
# when only the prompt string changes, check that. If the user's
# configuration turns out to send OSC 7 too, remove this hook.
__runner_osc7() {
  builtin local status=$? hooks="${PROMPT_COMMAND[*]-}|${precmd_functions[*]-}" roots= i
  if [[ $hooks != "${__runner_osc7_hooks-}" ]]; then
    __runner_osc7_seen=' '
    roots="$hooks ${PS1-}"
  elif [[ ${PS1-} != "${__runner_osc7_prompt-}" ]]; then
    roots=${PS1-}
  fi
  __runner_osc7_hooks=$hooks
  __runner_osc7_prompt=${PS1-}
  if [[ -n $roots ]] && __runner_osc7_sent_elsewhere "$roots"; then
    if [[ "$(builtin declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
      for i in "${!PROMPT_COMMAND[@]}"; do
        __runner_osc7_drop "${PROMPT_COMMAND[i]}"
        PROMPT_COMMAND[i]=$__runner_osc7_entry
      done
    else
      __runner_osc7_drop "${PROMPT_COMMAND-}"
      PROMPT_COMMAND=$__runner_osc7_entry
    fi
    builtin unset __runner_osc7_entry
    return $status
  fi
  builtin local LC_ALL=C dir="$PWD"
  if [[ $dir == *[^A-Za-z0-9/._~-]* ]]; then
    builtin local c hex encoded=
    for (( i = 0; i < ${#dir}; i++ )); do
      c=${dir:i:1}
      if [[ $c == [A-Za-z0-9/._~-] ]]; then
        encoded+=$c
      else
        builtin printf -v hex '%d' "'$c"
        builtin printf -v hex '%%%02X' $(( hex & 255 ))
        encoded+=$hex
      fi
    done
    dir=$encoded
  fi
  # No host: this hook only runs in a local shell, and $HOSTNAME, read once
  # at startup, goes stale when a laptop's hostname follows the network.
  builtin printf '\033]7;file://%s\033\\' "$dir"
  return $status
}

__runner_osc7_bootstrap='. "$RUNNER_BASH_INTEGRATION"'
if [[ "$(builtin declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND=("${PROMPT_COMMAND[@]//"$__runner_osc7_bootstrap"/__runner_osc7}")
else
  PROMPT_COMMAND=${PROMPT_COMMAND//"$__runner_osc7_bootstrap"/__runner_osc7}
fi
builtin export -n PROMPT_COMMAND
builtin unset RUNNER_BASH_INTEGRATION __runner_osc7_bootstrap
__runner_osc7
builtin eval "builtin unset __runner_osc7_status; builtin return $__runner_osc7_status"
