import { ChevronDown } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

type CompactDropdownOption<T extends string> = {
  label: string;
  value: T;
};

type DropdownPosition = {
  left: number;
  maxHeight: number;
  top: number;
  width: number;
};

type CompactDropdownProps<T extends string> = {
  className: string;
  icon?: ReactNode;
  label: string;
  onChange: (value: T) => void;
  options: CompactDropdownOption<T>[];
  value: T;
};

export function CompactDropdown<T extends string>({ className, icon, label, onChange, options, value }: CompactDropdownProps<T>) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<DropdownPosition | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const selectedLabel = options.find((option) => option.value === value)?.label ?? label;

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewportGap = 8;
    const menuGap = 5;
    const rowHeight = 30;
    const longestLabel = options.reduce((longest, option) => Math.max(longest, option.label.length), label.length);
    const contentWidth = Math.min(240, Math.max(112, rect.width, longestLabel * 8 + 44));
    const naturalHeight = Math.min(236, Math.max(38, options.length * rowHeight + 8));
    const spaceBelow = window.innerHeight - rect.bottom - viewportGap;
    const spaceAbove = rect.top - viewportGap;
    const openBelow = spaceBelow >= Math.min(naturalHeight, 112) || spaceBelow >= spaceAbove;
    const maxHeight = Math.max(38, Math.min(naturalHeight, openBelow ? spaceBelow : spaceAbove));
    const top = openBelow ? rect.bottom + menuGap : rect.top - maxHeight - menuGap;
    const left = Math.min(Math.max(viewportGap, rect.left), window.innerWidth - contentWidth - viewportGap);
    setPosition({ left, maxHeight, top: Math.max(viewportGap, top), width: contentWidth });
  }, [label.length, options]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, updatePosition]);

  const selectOption = (nextValue: T) => {
    onChange(nextValue);
    setOpen(false);
    triggerRef.current?.focus();
  };

  return (
    <div className={`compact-dropdown ${className}`}>
      <button
        className="compact-dropdown-trigger"
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        title={label}
        ref={triggerRef}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        {icon}
        <span className="compact-dropdown-value">{selectedLabel}</span>
        <ChevronDown size={12} />
      </button>
      {open && position && createPortal(
        <div
          className="compact-dropdown-menu"
          role="listbox"
          aria-label={label}
          ref={menuRef}
          style={{ left: position.left, maxHeight: position.maxHeight, top: position.top, width: position.width }}
        >
          {options.map((option) => (
            <button
              className="compact-dropdown-option"
              type="button"
              role="option"
              aria-selected={option.value === value}
              data-selected={option.value === value}
              key={option.value}
              onClick={() => selectOption(option.value)}
            >
              <span>{option.label}</span>
            </button>
          ))}
        </div>,
        document.body,
      )}
    </div>
  );
}
