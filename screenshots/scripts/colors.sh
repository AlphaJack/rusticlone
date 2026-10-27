# The web UI's colors in assets/style.css, as hex, for the terminal and the notifications; the blue, purple and cyan
# it lacks are of the same Tailwind palette as its green, yellow and red.
# colors SCHEME: sets them for the dark or light color scheme.
colors() {
  if [ "$1" = dark ]; then
    background=#161718 card=#1a1b1c foreground=#f7f7f8 muted=#a8aaaf border=#2a2b2d
    red=#ef4444 green=#22c55e yellow=#eab308 blue=#3b82f6 purple=#a855f7 cyan=#06b6d4
  else
    background=#faf9f8 card=#ffffff foreground=#1a1a1a muted=#64615e border=#dddad7
    red=#dc2626 green=#16a34a yellow=#ca8a04 blue=#2563eb purple=#9333ea cyan=#0891b2
  fi
}

# theme SCHEME: the terminal theme, in vhs's JSON, whose bright colors are the same as the others.
theme() {
  colors "$1"
  local json="\"name\":\"Rusticlone\",\"background\":\"$card\",\"foreground\":\"$foreground\",\"cursor\":\"$foreground\",\"selection\":\"$border\""
  local pair
  for pair in Black:$muted Red:$red Green:$green Yellow:$yellow Blue:$blue Purple:$purple Cyan:$cyan White:$muted; do
    json+=",\"$(echo "${pair%%:*}" | tr A-Z a-z)\":\"${pair#*:}\",\"bright${pair%%:*}\":\"${pair#*:}\""
  done
  echo "{$json}"
}
