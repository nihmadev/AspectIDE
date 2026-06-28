import { ChevronDown, ChevronRight, EyeOff, Search } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

type CompactDropdownOption<T extends string> = {
  label: string;
  value: T;
  group?: string;
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
  /** When set (and there are enough options), show a live filter box at the top. */
  searchable?: boolean;
  /** Minimum option count before the search box appears (default 7). */
  searchThreshold?: number;
  searchPlaceholder?: string;
  /** Localized empty-state shown when a search yields no matches ({query} interpolated by caller). */
  searchEmptyLabel?: string;
  /** When provided, each option shows a "hide" affordance that calls this. */
  onHideOption?: (value: T) => void;
  hideOptionLabel?: string;
  /** Optional node rendered pinned at the bottom of the menu (e.g. "Show N hidden"). */
  footer?: ReactNode;
};

const ROW_HEIGHT = 30;
const GROUP_HEADER_HEIGHT = 32;
const SEARCH_ROW_HEIGHT = 38;
const FOOTER_ROW_HEIGHT = 34;
const MENU_MAX_HEIGHT = 360;
const MENU_MIN_HEIGHT = 38;
const MENU_MIN_WIDTH = 112;
const MENU_MAX_WIDTH = 340;
const MENU_HORIZONTAL_PADDING = 44;
const LABEL_WIDTH_FACTOR = 8;

export function CompactDropdown<T extends string>({
  className,
  icon,
  label,
  onChange,
  options,
  value,
  searchable = false,
  searchThreshold = 7,
  searchPlaceholder,
  searchEmptyLabel,
  onHideOption,
  hideOptionLabel,
  footer,
}: CompactDropdownProps<T>) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<DropdownPosition | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set());
  const [query, setQuery] = useState("");
  const [activeValue, setActiveValue] = useState<T | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const activeRowRef = useRef<HTMLDivElement | null>(null);
  const selectedLabel = options.find((option) => option.value === value)?.label ?? label;

  const showSearch = searchable && options.length >= searchThreshold;
  const filteredOptions = useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (!trimmed) return options;
    return options.filter((option) =>
      `${option.label} ${option.group ?? ""}`.toLowerCase().includes(trimmed));
  }, [options, query]);

  const groupedOptions = useMemo(() => groupOptions(filteredOptions), [filteredOptions]);
  // Flattened, in-display-order list of currently visible option values (respecting
  // collapsed groups) — drives arrow-key navigation.
  const navigableValues = useMemo(() => {
    const result: T[] = [];
    for (const group of groupedOptions) {
      if (collapsedGroups.has(group.name)) continue;
      for (const option of group.options) result.push(option.value);
    }
    return result;
  }, [groupedOptions, collapsedGroups]);

  const visibleOptionsCount = groupedOptions.reduce((count, group) => (
    count + (groupedOptions.length > 1 ? 1 : 0) + (collapsedGroups.has(group.name) ? 0 : group.options.length)
  ), 0);

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const viewportGap = 8;
    const menuGap = 5;
    const longestLabel = options.reduce((longest, option) => Math.max(longest, option.label.length, option.group?.length ?? 0), label.length);
    const contentWidth = Math.min(MENU_MAX_WIDTH, Math.max(MENU_MIN_WIDTH, rect.width, longestLabel * LABEL_WIDTH_FACTOR + MENU_HORIZONTAL_PADDING));
    const chrome = (showSearch ? SEARCH_ROW_HEIGHT : 0) + (footer ? FOOTER_ROW_HEIGHT : 0);
    const naturalHeight = Math.min(MENU_MAX_HEIGHT, Math.max(MENU_MIN_HEIGHT, visibleOptionsCount * ROW_HEIGHT + chrome + 8));
    const spaceBelow = window.innerHeight - rect.bottom - viewportGap;
    const spaceAbove = rect.top - viewportGap;
    const openBelow = spaceBelow >= Math.min(naturalHeight, 112) || spaceBelow >= spaceAbove;
    const maxHeight = Math.max(MENU_MIN_HEIGHT, Math.min(naturalHeight, openBelow ? spaceBelow : spaceAbove));
    const top = openBelow ? rect.bottom + menuGap : rect.top - maxHeight - menuGap;
    const left = Math.min(Math.max(viewportGap, rect.left), window.innerWidth - contentWidth - viewportGap);
    setPosition({ left, maxHeight, top: Math.max(viewportGap, top), width: contentWidth });
  }, [label.length, options, visibleOptionsCount, showSearch, footer]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, updatePosition, collapsedGroups]);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setActiveValue(null);
      return;
    }
    // Focus the search box when present; otherwise focus the menu itself so arrow-key
    // navigation and Escape work for non-searchable dropdowns too.
    requestAnimationFrame(() => {
      if (showSearch) searchRef.current?.focus();
      else menuRef.current?.focus();
    });
  }, [open, showSearch]);

  // Keep the active row visible while arrow-key navigating a long/grouped list.
  useEffect(() => {
    if (!open || activeValue == null) return;
    activeRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [open, activeValue]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [open, updatePosition]);

  const selectOption = (nextValue: T) => {
    onChange(nextValue);
    setOpen(false);
    triggerRef.current?.focus();
  };

  const toggleGroup = (groupName: string) => {
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (next.has(groupName)) next.delete(groupName);
      else next.add(groupName);
      return next;
    });
  };

  const moveActive = (delta: number) => {
    if (navigableValues.length === 0) return;
    const currentIndex = activeValue ? navigableValues.indexOf(activeValue) : -1;
    const nextIndex = (currentIndex + delta + navigableValues.length) % navigableValues.length;
    setActiveValue(navigableValues[nextIndex]);
  };

  const handleMenuKeyDown = (event: { key: string; preventDefault: () => void }) => {
    if (event.key === "Escape") {
      setOpen(false);
      triggerRef.current?.focus();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const target = activeValue ?? navigableValues[0];
      if (target) selectOption(target);
    }
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
          aria-activedescendant={activeValue ? `cdo-${className}-${activeValue}` : undefined}
          tabIndex={-1}
          ref={menuRef}
          style={{ left: position.left, maxHeight: position.maxHeight, top: position.top, width: position.width }}
          onKeyDown={handleMenuKeyDown}
        >
          {showSearch && (
            <div className="compact-dropdown-search">
              <Search size={13} aria-hidden="true" />
              <input
                ref={searchRef}
                type="text"
                value={query}
                placeholder={searchPlaceholder ?? "Search…"}
                aria-label={searchPlaceholder ?? "Search"}
                onChange={(event) => { setQuery(event.target.value); setActiveValue(null); }}
              />
            </div>
          )}
          <div className="compact-dropdown-scroll">
            {groupedOptions.length === 0 && (
              <div className="compact-dropdown-empty">{query.trim() ? (searchEmptyLabel ?? `No matches for "${query.trim()}"`) : "—"}</div>
            )}
            {groupedOptions.map((group) => {
              const collapsed = collapsedGroups.has(group.name);
              return (
                <div className="compact-dropdown-group" key={group.name}>
                  {groupedOptions.length > 1 && (
                    <button
                      className="compact-dropdown-group-header"
                      type="button"
                      aria-expanded={!collapsed}
                      onClick={() => toggleGroup(group.name)}
                      style={{ minHeight: GROUP_HEADER_HEIGHT }}
                    >
                      {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
                      <span>{group.name}</span>
                      <small>{group.options.length}</small>
                    </button>
                  )}
                  {!collapsed && group.options.map((option) => (
                    <div
                      className="compact-dropdown-option-row"
                      data-selected={option.value === value}
                      data-active={option.value === activeValue}
                      key={option.value}
                      ref={option.value === activeValue ? activeRowRef : undefined}
                    >
                      <button
                        className="compact-dropdown-option"
                        type="button"
                        role="option"
                        id={`cdo-${className}-${option.value}`}
                        aria-selected={option.value === value}
                        onClick={() => selectOption(option.value)}
                        onMouseEnter={() => setActiveValue(option.value)}
                      >
                        <span>{option.label}</span>
                      </button>
                      {onHideOption && option.value !== value && (
                        <button
                          className="compact-dropdown-option-hide"
                          type="button"
                          aria-label={hideOptionLabel ?? "Hide"}
                          title={hideOptionLabel ?? "Hide"}
                          onClick={(event) => { event.stopPropagation(); onHideOption(option.value); }}
                        >
                          <EyeOff size={12} />
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
          {footer && <div className="compact-dropdown-footer">{footer}</div>}
        </div>,
        document.body,
      )}
    </div>
  );
}

function groupOptions<T extends string>(options: CompactDropdownOption<T>[]) {
  const groups = new Map<string, CompactDropdownOption<T>[]>();
  for (const option of options) {
    const groupName = option.group?.trim() || "Models";
    groups.set(groupName, [...(groups.get(groupName) ?? []), option]);
  }
  return Array.from(groups, ([name, groupOptions]) => ({ name, options: groupOptions }));
}
