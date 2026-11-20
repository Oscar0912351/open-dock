export function Kbd({ keys }: { keys: string }) {
  const parts = keys
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);

  return (
    <span className="cc-kbd-group">
      {parts.map((part, idx) => (
        <kbd key={`${part}-${idx}`} className="cc-kbd">
          {part}
        </kbd>
      ))}
    </span>
  );
}
