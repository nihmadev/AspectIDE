import { ChevronDown, ChevronRight, EyeOff, Search } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

type CompactDropdownOption<T extends string> = {
  label: string;
  value: T;
  group?: string;
  /** Trailing badge text shown right-aligned in the option row (e.g. "0/1000"). */
  badge?: string;
  /** Leading availability dot: "ok" (green) or "blocked" (red). */
  status?: "ok" | "blocked";
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
  disabled?: boolean;
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
  /** Per-option inline style (also applied to the trigger's current value) — e.g. font pickers previewing each family in itself. */
  getOptionStyle?: (value: T) => CSSProperties | undefined;
  /** Optional node rendered in the TRIGGER as a quiet second line BELOW the
   *  selected label (not in the option list) — e.g. the provider name shown
   *  under the model, centered. Turns the trigger into a two-line stacked box. */
  triggerSubLabel?: ReactNode;
};

// Session-scoped view memory per dropdown (keyed by className + label): the
// scroll offset and collapsed groups survive close/reopen AND component
// remounts (panel switches, chat re-open) — refs alone die with the instance.
type DropdownViewMemory = { scrollTop: number | null; collapsed: ReadonlySet<string> };
const dropdownViewMemory = new Map<string, DropdownViewMemory>();

function viewMemoryFor(key: string): DropdownViewMemory {
  let memory = dropdownViewMemory.get(key);
  if (!memory) {
    memory = { scrollTop: null, collapsed: new Set() };
    dropdownViewMemory.set(key, memory);
  }
  return memory;
}

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
  disabled = false,
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
  getOptionStyle,
  triggerSubLabel,
}: CompactDropdownProps<T>) {
  // Label is part of the key so the three composer selects (same className)
  // don't share one memory slot.
  const viewMemory = viewMemoryFor(`${className}::${label}`);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<DropdownPosition | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set(viewMemory.collapsed));
  const [query, setQuery] = useState("");
  const [activeValue, setActiveValue] = useState<T | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);
  const activeRowRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  // Inside a modal dialog (Radix) the menu must portal into the dialog content,
  // not document.body: a body-level portal counts as "outside" for the dialog's
  // dismiss/focus/scroll-lock layers — clicking an option would close the whole
  // dialog and the scroll lock would swallow wheel events over the list.
  const portalHostRef = useRef<HTMLElement | null>(null);
  const selectedRowRef = useRef<HTMLDivElement | null>(null);
  const scrollRestoredRef = useRef(false);
  // Auto-scroll follows the active row only for keyboard navigation; hovering while
  // the wheel is moving must never yank the list around.
  const keyboardNavRef = useRef(false);
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
    const host = portalHostRef.current ?? document.body;
    const hosted = host !== document.body;
    // Bounds the menu must stay within: the viewport, or the host dialog's box
    // (its overflow/paint containment would clip anything past the edge anyway).
    const clip = hosted
      ? host.getBoundingClientRect()
      : { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight };
    const viewportGap = 8;
    const menuGap = 5;
    const longestLabel = options.reduce((longest, option) => Math.max(longest, option.label.length, option.group?.length ?? 0), label.length);
    const contentWidth = Math.min(MENU_MAX_WIDTH, Math.max(MENU_MIN_WIDTH, rect.width, longestLabel * LABEL_WIDTH_FACTOR + MENU_HORIZONTAL_PADDING));
    const chrome = (showSearch ? SEARCH_ROW_HEIGHT : 0) + (footer ? FOOTER_ROW_HEIGHT : 0);
    const naturalHeight = Math.min(MENU_MAX_HEIGHT, Math.max(MENU_MIN_HEIGHT, visibleOptionsCount * ROW_HEIGHT + chrome + 8));
    const spaceBelow = clip.bottom - rect.bottom - viewportGap;
    const spaceAbove = rect.top - clip.top - viewportGap;
    const openBelow = spaceBelow >= Math.min(naturalHeight, 112) || spaceBelow >= spaceAbove;
    const maxHeight = Math.max(MENU_MIN_HEIGHT, Math.min(naturalHeight, openBelow ? spaceBelow : spaceAbove));
    const top = Math.max(clip.top + viewportGap, openBelow ? rect.bottom + menuGap : rect.top - maxHeight - menuGap);
    const left = Math.min(Math.max(clip.left + viewportGap, rect.left), clip.right - contentWidth - viewportGap);
    // Hosted menus are absolutely positioned inside the (positioned) dialog, so
    // viewport coordinates shift into the host's padding-box space.
    const offsetX = hosted ? clip.left + host.clientLeft : 0;
    const offsetY = hosted ? clip.top + host.clientTop : 0;
    setPosition({
      left: left - offsetX,
      maxHeight,
      top: top - offsetY,
      width: contentWidth,
    });
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
  // Mouse-driven activation (hover) is excluded on purpose — see keyboardNavRef.
  useEffect(() => {
    if (!open || activeValue == null || !keyboardNavRef.current) return;
    keyboardNavRef.current = false;
    activeRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [open, activeValue]);

  // Restore the remembered scroll offset once per open (after the portal has laid
  // out); fall back to centering the current selection on the very first open.
  useLayoutEffect(() => {
    if (!open || !position) {
      scrollRestoredRef.current = false;
      return;
    }
    if (scrollRestoredRef.current) return;
    scrollRestoredRef.current = true;
    const scroller = scrollRef.current;
    if (!scroller) return;
    if (viewMemory.scrollTop != null) {
      scroller.scrollTop = viewMemory.scrollTop;
      return;
    }
    // Center the selection manually — scrollIntoView could also scroll outer
    // containers (e.g. the settings page behind a hosted menu).
    const row = selectedRowRef.current;
    if (!row) return;
    const rowRect = row.getBoundingClientRect();
    const scrollerRect = scroller.getBoundingClientRect();
    scroller.scrollTop += rowRect.top - scrollerRect.top - (scroller.clientHeight - rowRect.height) / 2;
  }, [open, position]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    // Reposition when the surrounding UI scrolls — but the menu's own list
    // scrolling must not churn position state on every wheel tick.
    const handleScroll = (event: Event) => {
      if (menuRef.current?.contains(event.target as Node | null)) return;
      updatePosition();
    };
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", handleScroll, true);
    window.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", handleScroll, true);
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
      viewMemory.collapsed = next;
      return next;
    });
  };

  const moveActive = (delta: number) => {
    if (navigableValues.length === 0) return;
    const currentIndex = activeValue ? navigableValues.indexOf(activeValue) : -1;
    const nextIndex = (currentIndex + delta + navigableValues.length) % navigableValues.length;
    keyboardNavRef.current = true;
    setActiveValue(navigableValues[nextIndex]);
  };

  const handleMenuKeyDown = (event: { key: string; preventDefault: () => void; stopPropagation: () => void }) => {
    if (event.key === "Escape") {
      // Close only the menu — inside a dialog the bubbled Escape would
      // otherwise dismiss the whole dialog too.
      event.stopPropagation();
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
        disabled={disabled}
        onClick={() => {
          if (disabled) return;
          portalHostRef.current = triggerRef.current?.closest<HTMLElement>("[role=\"dialog\"]") ?? null;
          setOpen((current) => !current);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            portalHostRef.current = triggerRef.current?.closest<HTMLElement>("[role=\"dialog\"]") ?? null;
            setOpen(true);
          }
        }}
      >
        {icon}
        {triggerSubLabel ? (
          <span className="compact-dropdown-stack">
            <span className="compact-dropdown-value" style={getOptionStyle?.(value)}>{selectedLabel}</span>
            <span className="compact-dropdown-sublabel">{triggerSubLabel}</span>
          </span>
        ) : (
          <span className="compact-dropdown-value" style={getOptionStyle?.(value)}>{selectedLabel}</span>
        )}
        <ChevronDown size={12} />
      </button>
      {open && position && createPortal(
        <div
          className="compact-dropdown-menu"
          data-hosted={portalHostRef.current ? "true" : undefined}
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
          <div
            className="compact-dropdown-scroll"
            ref={scrollRef}
            onScroll={(event) => {
              // Only unfiltered positions are worth remembering — an offset
              // inside search results is meaningless once the query resets.
              if (!query.trim()) viewMemory.scrollTop = event.currentTarget.scrollTop;
            }}
          >
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
                      ref={option.value === activeValue || option.value === value ? (node) => {
                        if (option.value === activeValue) activeRowRef.current = node;
                        if (option.value === value) selectedRowRef.current = node;
                      } : undefined}
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
                        {option.status && (
                          <span
                            className={`compact-dropdown-dot compact-dropdown-dot-${option.status}`}
                            title={option.status === "blocked" ? "Limit reached" : "Available"}
                            aria-hidden="true"
                          />
                        )}
                        <span style={getOptionStyle?.(option.value)}>{option.label}</span>
                        {option.badge && <span className="compact-dropdown-badge">{option.badge}</span>}
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
        portalHostRef.current ?? document.body,
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
