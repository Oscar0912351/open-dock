// Parser for "pasta 10" → { label: "Pasta", durationMs: 600000 }.
// Ren funktion — ingen React, kan unit-testes. Se timer-3-spec for reglerne.

export interface ParsedTimer {
  label: string;
  durationMs: number;
}

const MIN_MS = 1_000;
const MAX_MS = 86_400_000;
const MAX_LABEL_CHARS = 60;

// `\b` kender ikke æøå — derfor lookaround på alle bogstaver/cifre.
// 1. Klokkeslæt: "kl 14:30", "kl. 15.15", "kl 9"
const RE_CLOCK = /(?<![\p{L}\p{N}])kl\.?\s*(\d{1,2})(?:[:.](\d{2}))?(?![\p{L}\p{N}])/iu;
// 2a. Sammenblandet tid+enhed: "1t30", "1t 30m", "2h15"
const RE_HM = /(?<![\p{L}\p{N}])(\d+)\s*[th]\s*(\d{1,2})\s*(?:m|min)?(?![\p{L}\p{N}])/giu;
// 2b. Kolon-tid: "2:30" = 2 min 30 s, "1:00:00" = 1 t. mm:ss altid < 60.
const RE_COLON = /(?<![\p{L}\p{N}])(\d{1,2}):([0-5]\d)(?::([0-5]\d))?(?![\p{L}\p{N}])/gu;
// 2c. Tal + enhed: "10 min", "1,5 t", "90s", "45 sek"
const RE_UNIT =
  /(?<![\p{L}\p{N}])(\d+(?:[.,]\d+)?)\s*(timer|time|t|h|minutter|minut|min|m|sekunder|sekund|sek|s)(?![\p{L}\p{N}])/giu;

const UNIT_MS: Record<string, number> = {
  timer: 3_600_000,
  time: 3_600_000,
  t: 3_600_000,
  h: 3_600_000,
  minutter: 60_000,
  minut: 60_000,
  min: 60_000,
  m: 60_000,
  sekunder: 1_000,
  sekund: 1_000,
  sek: 1_000,
  s: 1_000,
};

const EDGE_WORDS = new Set(["om", "i", "på", "kl"]);

/** Fjerner alle matches af `re` fra `text` og returnerer rest + summen. */
function eat(
  text: string,
  re: RegExp,
  toMs: (match: RegExpExecArray) => number,
): { rest: string; sum: number; found: boolean } {
  re.lastIndex = 0;
  const spans: Array<[number, number]> = [];
  let sum = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    spans.push([match.index, match.index + match[0].length]);
    sum += toMs(match);
    if (match[0].length === 0) re.lastIndex += 1;
  }
  if (spans.length === 0) return { rest: text, sum: 0, found: false };
  let out = "";
  let last = 0;
  for (const [start, end] of spans) {
    out += `${text.slice(last, start)} `;
    last = end;
  }
  out += text.slice(last);
  return { rest: out, sum, found: true };
}

function bareWord(word: string): string {
  return word.replace(/^[-–:,.]+|[-–:,.]+$/g, "").toLowerCase();
}

/** Resterende tekst → timer-navn: kantord væk, mellemrum samles, stort forbogstav, max 60 tegn. */
export function cleanTimerLabel(raw: string): string {
  let text = raw.replace(/\s+/g, " ").trim();
  text = text.replace(/^[-–:,.]+/, "").replace(/[-–:,.]+$/, "");
  const words = text.split(" ").filter((word) => word.length > 0);
  while (words.length > 0 && EDGE_WORDS.has(bareWord(words[0] ?? ""))) words.shift();
  while (words.length > 0 && EDGE_WORDS.has(bareWord(words[words.length - 1] ?? ""))) words.pop();
  const joined = words
    .join(" ")
    .replace(/^[-–:,.]+/, "")
    .replace(/[-–:,.]+$/, "");
  if (!joined) return "";
  const chars = Array.from(joined);
  const capped = chars.slice(0, MAX_LABEL_CHARS).join("");
  return capped.charAt(0).toLocaleUpperCase("da-DK") + capped.slice(1);
}

/**
 * Parser "pasta 10" → { label: "Pasta", durationMs: 600_000 }.
 * Returnerer `null` når ingen gyldig tid findes eller varigheden er uden for
 * 1 s – 24 t. `now` bruges til at placere "kl 14:30" på rigtig dato.
 */
export function parseTimerInput(raw: string, now: Date): ParsedTimer | null {
  const text = raw.trim();
  if (!text) return null;

  // 1. Klokkeslæt — når det findes, parses ingen andre tider.
  RE_CLOCK.lastIndex = 0;
  const clock = RE_CLOCK.exec(text);
  if (clock) {
    const hours = Number(clock[1]);
    const minutes = clock[2] !== undefined ? Number(clock[2]) : 0;
    if (hours <= 23 && minutes <= 59) {
      const target = new Date(now.getTime());
      target.setHours(hours, minutes, 0, 0);
      if (target.getTime() <= now.getTime()) {
        target.setDate(target.getDate() + 1);
      }
      const durationMs = Math.round(target.getTime() - now.getTime());
      const rest = `${text.slice(0, clock.index)} ${text.slice(clock.index + clock[0].length)}`;
      if (durationMs >= MIN_MS && durationMs <= MAX_MS) {
        return { label: cleanTimerLabel(rest), durationMs };
      }
      return null;
    }
    // Ugyldigt klokkeslæt (fx "kl 25") → læs resten som almindelig tekst.
  }

  // 2. Tidsenheder i rækkefølge — hver matcher fjernes og varighederne summères.
  let rest = text;
  let durationMs = 0;
  let found = false;

  const hm = eat(rest, RE_HM, (m) => (Number(m[1]) * 3600 + Number(m[2]) * 60) * 1000);
  rest = hm.rest;
  durationMs += hm.sum;
  found = found || hm.found;

  const colon = eat(rest, RE_COLON, (m) => {
    const first = Number(m[1]);
    const second = Number(m[2]);
    const third = m[3];
    if (third !== undefined) return (first * 3600 + second * 60 + Number(third)) * 1000;
    return (first * 60 + second) * 1000;
  });
  rest = colon.rest;
  durationMs += colon.sum;
  found = found || colon.found;

  const unit = eat(rest, RE_UNIT, (m) => {
    const value = Number((m[1] ?? "0").replace(",", "."));
    return value * (UNIT_MS[(m[2] ?? "").toLowerCase()] ?? 0);
  });
  rest = unit.rest;
  durationMs += unit.sum;
  found = found || unit.found;

  // 2d. Kun hvis a–c intet fandt: et rent tal som sidste (ellers første) ord = minutter.
  if (!found) {
    const words = rest.split(/\s+/).filter((word) => word.length > 0);
    const isNumber = (word: string) => /^\d+(?:[.,]\d+)?$/.test(word);
    let index = -1;
    if (words.length > 0 && isNumber(words[words.length - 1] ?? "")) {
      index = words.length - 1;
    } else if (words.length > 0 && isNumber(words[0] ?? "")) {
      index = 0;
    }
    if (index >= 0) {
      durationMs = Number((words[index] ?? "0").replace(",", ".")) * 60_000;
      found = true;
      words.splice(index, 1);
      rest = words.join(" ");
    }
  }

  if (!found) return null;
  durationMs = Math.round(durationMs);
  if (durationMs < MIN_MS || durationMs > MAX_MS) return null;
  return { label: cleanTimerLabel(rest), durationMs };
}
