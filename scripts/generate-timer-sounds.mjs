#!/usr/bin/env node
// Syntetiserer timer-widgettens tre premium-lyde som 44.1 kHz 16-bit stereo WAV.
// Ingen dependencies — ren additiv syntese. Kør: node scripts/generate-timer-sounds.mjs
//
//  chime.wav   — blød meditationsklokke: inharmoniske partials, to slag (E5, så kvinten under)
//  marimba.wav — varm marimba-frase: C5–E5–G5 med træ-partials og mallet-transient
//  crystal.wav — glasagtig bloom: to toner en stor tert fra hinanden, langsom attack
//
// Alle normaliseres til −3 dBFS peak og ender med fade-out, så der aldrig er klik.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SR = 44100;
const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "resources", "sounds");

/** Render `dur` sekunder til stereo-floats [L, R] via en sample-fn. */
function render(seconds, sampleFn) {
  const n = Math.round(seconds * SR);
  const left = new Float64Array(n);
  const right = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const [l, r] = sampleFn(i / SR);
    left[i] = l;
    right[i] = r;
  }
  return { left, right, n };
}

/** Ét partial-led: sinus med eksponentielt henfald og blød attack. */
function partial(t, t0, freq, amp, decayS, attackS = 0.004) {
  const x = t - t0;
  if (x < 0) return 0;
  const env = Math.exp(-x / decayS) * Math.min(1, x / attackS);
  return amp * env * Math.sin(2 * Math.PI * freq * x);
}

/** Normaliser til peak -3 dBFS og fade de sidste `fadeMs` ud. Returnerer Int16 stereo interleaved. */
function finalize({ left, right, n }, fadeMs = 120) {
  let peak = 0;
  for (let i = 0; i < n; i++) {
    peak = Math.max(peak, Math.abs(left[i]), Math.abs(right[i]));
  }
  const target = Math.pow(10, -3 / 20); // −3 dBFS
  const gain = peak > 0 ? target / peak : 1;
  const fadeN = Math.round((fadeMs / 1000) * SR);
  const out = new Int16Array(n * 2);
  for (let i = 0; i < n; i++) {
    const fade = i >= n - fadeN ? (n - i) / fadeN : 1;
    out[i * 2] = Math.max(-32768, Math.min(32767, Math.round(left[i] * gain * fade * 32767)));
    out[i * 2 + 1] = Math.max(-32768, Math.min(32767, Math.round(right[i] * gain * fade * 32767)));
  }
  return out;
}

function writeWav(name, pcm) {
  const dataSize = pcm.length * 2;
  const buf = Buffer.alloc(44 + dataSize);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(2, 22); // stereo
  buf.writeUInt32LE(SR, 24);
  buf.writeUInt32LE(SR * 4, 28);
  buf.writeUInt16LE(4, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(dataSize, 40);
  for (let i = 0; i < pcm.length; i++) buf.writeInt16LE(pcm[i], 44 + i * 2);
  mkdirSync(OUT_DIR, { recursive: true });
  writeFileSync(join(OUT_DIR, name), buf);
  console.log(`${name}: ${(buf.length / 1024).toFixed(1)} kB`);
}

// ─── chime.wav — meditationsklokke ─────────────────────────────────────────
// Klokke-partials (midt-oktav-afstemning): hum 0.5, prime 1, tierce 1.19/2.01,
// quint 2.74, nominal 3.76, deciem 5.4. Høje partials henfalder hurtigere.
function bellStrike(t, t0, fundamental, amp, pan = 0) {
  const partials = [
    [1.0, 1.0, 3.4],
    [2.01, 0.42, 2.1],
    [1.19, 0.18, 1.6],
    [2.74, 0.22, 1.25],
    [3.76, 0.12, 0.8],
    [5.4, 0.06, 0.5],
  ];
  let sum = 0;
  for (const [ratio, a, decay] of partials) {
    sum += partial(t, t0, fundamental * ratio, a, decay);
  }
  // Ørestykket bredde: en anelse detune forskel pr. kanal.
  const spread = 1 + pan * 0.0006;
  return amp * sum * spread;
}

const chime = render(5.2, (t) => {
  const E5 = 659.26;
  const A4 = 440.0; // kvinten under
  const strike1 = bellStrike(t, 0, E5, 0.9);
  const strike2 = bellStrike(t, 1.05, A4, 0.55);
  // Antiphon: kanalerne spejler hinanden en smule for rumklang.
  const l = strike1 * (1 - 0.0006) + strike2 * (1 + 0.0006);
  const r = strike1 * (1 + 0.0006) + strike2 * (1 - 0.0006);
  return [l, r];
});
writeWav("chime.wav", finalize(chime));

// ─── marimba.wav — varm træ-frase C5–E5–G5 ─────────────────────────────────
// Marimba-partials: fundamental, ~3.9× (kvint+oktav), ~9.8×. Kort attack,
// medium henfald. Mallet-transient: dæmpet støj i de første 15 ms.
function mulberry32(seed) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let z = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    z = (z + Math.imul(z ^ (z >>> 7), 61 | z)) ^ z;
    return ((z ^ (z >>> 14)) >>> 0) / 4294967296;
  };
}
const noise = mulberry32(1234567);

function marimbaNote(t, t0, freq, amp) {
  const x = t - t0;
  if (x < 0) return 0;
  let v = 0;
  v += partial(t, t0, freq, 1.0, 0.55, 0.002);
  v += partial(t, t0, freq * 3.92, 0.16, 0.16, 0.002);
  v += partial(t, t0, freq * 9.83, 0.05, 0.07, 0.002);
  // Mallet-anslag: kort lavpasset støjtransient (simpel 1. ordens dæmpning).
  if (x < 0.015) {
    v += (noise() * 2 - 1) * 0.18 * Math.exp(-x / 0.004);
  }
  return amp * v;
}

const marimba = render(2.2, (t) => {
  const C5 = 523.25;
  const E5 = 659.26;
  const G5 = 783.99;
  const n1 = marimbaNote(t, 0, C5, 0.85);
  const n2 = marimbaNote(t, 0.21, E5, 0.8);
  const n3 = marimbaNote(t, 0.42, G5, 0.9);
  // Sidste tone lidt til højre for et åbent, opløftet slutpunkt.
  return [n1 + n2 + n3 * 0.92, n1 + n2 + n3 * 1.04];
});
writeWav("marimba.wav", finalize(marimba));

// ─── crystal.wav — glas-bloom, stor tert ────────────────────────────────────
// To rene toner (A5 + C#6) med langsom attack og lang shimmer-hale.
// En svag 3. harmonisk giver glans; en anelse detune mellem kanaler giver svaj.
function crystalTone(t, t0, freq, amp, attackS, detune) {
  const x = t - t0;
  if (x < 0) return 0;
  const attack = attackS;
  const env =
    Math.exp(-Math.max(0, x - attack) / 1.9) *
    (x < attack ? Math.pow(x / attack, 1.6) : 1);
  const f = freq * (1 + detune);
  return amp * env * (Math.sin(2 * Math.PI * f * x) + 0.14 * Math.sin(2 * Math.PI * f * 3 * x) + 0.05 * Math.sin(2 * Math.PI * f * 5.02 * x));
}

const crystal = render(4.0, (t) => {
  const A5 = 880.0;
  const CS6 = 1108.73;
  const l =
    crystalTone(t, 0, A5, 0.75, 0.08, -0.0004) +
    crystalTone(t, 0.34, CS6, 0.62, 0.1, -0.0005) +
    crystalTone(t, 0.95, A5 * 2, 0.3, 0.06, -0.0003);
  const r =
    crystalTone(t, 0, A5, 0.75, 0.08, 0.0004) +
    crystalTone(t, 0.34, CS6, 0.62, 0.1, 0.0005) +
    crystalTone(t, 0.95, A5 * 2, 0.3, 0.06, 0.0003);
  return [l, r];
});
writeWav("crystal.wav", finalize(crystal));

console.log("Færdig — lyde skrevet til src-tauri/resources/sounds/");
