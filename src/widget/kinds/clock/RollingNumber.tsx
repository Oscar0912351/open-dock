import { useEffect, useState } from "react";

interface RollingNumberProps {
  value: string;
}

export function RollingNumber({ value }: RollingNumberProps) {
  const [last, setLast] = useState(value);
  const [fromChars, setFromChars] = useState<ReadonlyMap<number, string>>(
    () => new Map(),
  );

  if (last !== value) {
    // Rul fra det ciffer der vises lige nu — ikke en ældre værdi — så et
    // nyt skift midt i en animation ikke udløser et dobbeltrul.
    const from = new Map<number, string>();
    Array.from(value).forEach((character, index) => {
      const oldCharacter = last[index] ?? "";
      if (/\d/.test(character) && oldCharacter !== character) {
        from.set(index, oldCharacter);
      }
    });
    setLast(value);
    setFromChars(from);
  }

  useEffect(() => {
    if (fromChars.size === 0) return;
    const timer = window.setTimeout(() => setFromChars(new Map()), 340);
    return () => window.clearTimeout(timer);
  }, [fromChars]);

  return (
    <span className="rolling-number" aria-label={value}>
      {Array.from(value).map((character, index) => {
        const fromChar = fromChars.get(index);
        if (fromChar === undefined) {
          return (
            <span className="rolling-static" aria-hidden="true" key={`${index}-${character}`}>
              {character}
            </span>
          );
        }
        return (
          <span className="rolling-digit" aria-hidden="true" key={`${index}-${character}`}>
            <span className="rolling-old">{fromChar}</span>
            <span className="rolling-new">{character}</span>
          </span>
        );
      })}
    </span>
  );
}
