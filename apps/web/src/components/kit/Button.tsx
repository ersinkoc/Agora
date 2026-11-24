import type { ButtonHTMLAttributes } from 'react';

// The one button (visual-language.md §3). `danger` is for a destructive action or one that lets an agent act without asking (ADR-0102);
// `danger-ghost` is the quieter destructive action in a row (Remove, Forget…). Always a real <button>: Enter and Space press it.
export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-ghost';
export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: 'sm' | 'md' };

export function Button({ variant = 'secondary', size = 'md', type = 'button', className = '', ...rest }: ButtonProps) {
  const cls = ['kit-btn', variant !== 'secondary' && `kit-btn--${variant}`, size === 'sm' && 'kit-btn--sm', className].filter(Boolean).join(' ');
  return <button type={type} data-variant={variant} className={cls} {...rest} />;
}
