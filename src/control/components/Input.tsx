import type { InputHTMLAttributes } from "react";

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  numeric?: boolean;
}

export function Input({ numeric = false, className = "", ...rest }: InputProps) {
  return (
    <input
      className={`cc-input${numeric ? " is-num" : ""}${className ? ` ${className}` : ""}`}
      {...rest}
    />
  );
}
