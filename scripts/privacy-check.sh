#!/usr/bin/env bash
# Controllo di riservatezza: NIENTE chiavi, email o dati personali nel repository.
#   scripts/privacy-check.sh              file tracciati + indice (usato dall'hook pre-commit)
#   scripts/privacy-check.sh --history    controlla anche tutta la storia e le email di autore/committer (hook pre-push)
#   scripts/privacy-check.sh --install    installa gli hook locali pre-commit e pre-push (gli hook non vengono mai pubblicati)
# I dati personali da cercare (utente, nome, email git, home) sono ricavati dal sistema: non sono scritti qui dentro.
set -uo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--install" ]]; then
  for h in pre-commit pre-push; do
    printf '#!/usr/bin/env bash\nexec "$(git rev-parse --show-toplevel)/scripts/privacy-check.sh" %s\n' "$([[ $h == pre-push ]] && echo --history)" > ".git/hooks/$h"
    chmod +x ".git/hooks/$h"
  done
  echo "Hook installati: pre-commit e pre-push."; exit 0
fi

U=$(id -un); NAME=$(getent passwd "$U" | cut -d: -f5 | cut -d, -f1)
GE=$(git config user.email || true)
APPID="it\\.${U}\\.(PixelHub|Cable)"                       # l'identificatore dell'app contiene lo username: ammesso

fail=0
bad() { echo "PRIVACY: $*" >&2; fail=1; }

# 1) file che non devono mai essere versionati
while IFS= read -r f; do
  case "$f" in
    *.pem|*.key|*.crt|*.p12|*.pfx|.env|.env.*|*/identity/*|*/settings.json|settings.json|docs/screenshots*/*) bad "file vietato: $f" ;;
  esac
done < <(git ls-files; git diff --cached --name-only --diff-filter=ACMR 2>/dev/null)

# 2) contenuto: chiavi private, email, dati personali (tracciati; con --history anche ogni commit)
scan() {  # $1 = rev oppure "" per l'albero di lavoro
  local rev="$1" out
  local g=(git grep -nIE); [[ -n "$rev" ]] && g=(git grep -nIE)
  local where=(); [[ -n "$rev" ]] && where=("$rev")
  out=$("${g[@]}" -e '-----BEGIN [A-Z ]*PRIVATE KEY-----' "${where[@]}" -- . ':!scripts/privacy-check.sh' 2>/dev/null) && bad "chiave privata${rev:+ in $rev}: $(echo "$out" | head -1 | cut -c1-90)"
  out=$("${g[@]}" -e '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' "${where[@]}" -- . ':!Cargo.lock' ':!scripts/privacy-check.sh' 2>/dev/null | grep -v 'noreply@anthropic\.com' | head -1)
  [[ -n "$out" ]] && bad "indirizzo email${rev:+ in $rev}: $(echo "$out" | cut -c1-90)"
  local term
  for term in "$U" "$NAME" "$GE" "$HOME"; do
    [[ ${#term} -lt 3 ]] && continue
    out=$("${g[@]}" -i -e "$(printf '%s' "$term" | sed 's/[][\.*^$/]/\\&/g')" "${where[@]}" -- . ':!Cargo.lock' ':!scripts/privacy-check.sh' 2>/dev/null \
          | sed -E "s/$APPID//Ig" | grep -iF -- "$term" | head -1)
    [[ -n "$out" ]] && bad "dato personale («${term:0:2}…»)${rev:+ in $rev}: $(echo "$out" | cut -c1-90)"
  done
}
scan ""

# 3) storia: ogni commit e le email di autore/committer
if [[ "${1:-}" == "--history" ]]; then
  while IFS= read -r c; do scan "$c"; done < <(git rev-list HEAD)
  while IFS= read -r e; do
    [[ -z "$e" || "$e" == *noreply* ]] || bad "email di autore/committer nella storia: ${e:0:3}…"
  done < <(git log --format='%ae%n%ce' | sort -u)
fi

[[ $fail == 0 ]] && echo "privacy-check: ok" || { echo "privacy-check: FALLITO: niente commit/push finché non è risolto." >&2; exit 1; }
