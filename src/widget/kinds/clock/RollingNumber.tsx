import { useEffect, useState } from "react";

interface RollingNumberProps {
  value: string;
}

export function RollingNumber({ value }: RollingNumberProps) {
  const [previous, setPrevious] = useState(value);

  useEffect(() => {
    if (previous === value) return;
    const timer = window.setTimeout(() => setPrevious(value), 340);
    return () => window.clearTimeout(timer);
  }, [previous, value]);

  return (
    <span className="rolling-number" aria-label={value}>
      {Array.from(value).map((character, index) => {
        const oldCharacter = previous[index] ?? "";
        const changed = /\d/.test(character) && oldCharacter !== character;
        if (!changed) {
          return (
            <span className="rolling-static" aria-hidden="true" key={`${index}-${character}`}>
              {character}
            </span>
          );
        }
        return (
          <span className="rolling-digit" aria-hidden="true" key={`${index}-${character}`}>
            <span className="rolling-old">{oldCharacter}</span>
            <span className="rolling-new">{character}</span>
          </span>
        );
      })}
    </span>
  );
}
