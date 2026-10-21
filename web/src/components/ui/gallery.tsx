// The pictures on a page, like a session's transcript, open in one
// lightbox, which steps from each to the next in the order the page shows
// them.

import { createContext, type ReactNode, useCallback, useContext, useMemo, useRef, useState } from "react";
import { Lightbox, type LightboxItem } from "./lightbox";

interface Pictures {
  /** The page shows a picture at `at`. Returns what takes it away. */
  add(at: Element, item: () => LightboxItem | undefined): () => void;
  /** Shows the picture at `at` in the lightbox. */
  open(at: Element): void;
}

const Pictures = createContext<Pictures | null>(null);

interface Picture {
  at: Element;
  item: LightboxItem;
}

/** Whether `a` comes before (-1) or after (1) `b` on the page. */
const byPlace = (a: Picture, b: Picture) =>
  a.at.compareDocumentPosition(b.at) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1;

/**
 * Its pictures (`useLightbox`) open in one lightbox, which steps through
 * them all in the order the page shows them, the ones that came in while it
 * was open too.
 */
export function Gallery({ children }: { children: ReactNode }) {
  const pictures = useRef(new Map<Element, () => LightboxItem | undefined>());
  const [, setChanged] = useState(0);
  const [open, setOpen] = useState<Picture>();
  const api = useMemo<Pictures>(
    () => ({
      add: (at, item) => {
        pictures.current.set(at, item);
        setChanged((n) => n + 1);
        return () => {
          pictures.current.delete(at);
          setChanged((n) => n + 1);
        };
      },
      open: (at) => {
        const item = pictures.current.get(at)?.();
        if (item) setOpen({ at, item });
      },
    }),
    [],
  );
  let list: Picture[] = [];
  let index = -1;
  if (open) {
    list = [...pictures.current]
      .flatMap(([at, get]) => {
        const item = get();
        return item ? [{ at, item }] : [];
      })
      .sort(byPlace);
    index = list.findIndex((p) => p.at === open.at);
    // Shown somewhere else now (a message that was sending, sent): the same picture.
    if (index < 0) index = list.findIndex((p) => p.item.src === open.item.src);
    if (index < 0) {
      list = [open];
      index = 0;
    }
  }
  return (
    <Pictures value={api}>
      {children}
      {open && (
        <Lightbox
          items={list.map((p) => p.item)}
          index={index}
          onIndex={(i) => {
            const p = list[i];
            if (p) setOpen(p);
          }}
          onClose={() => setOpen(undefined)}
        />
      )}
    </Pictures>
  );
}

/**
 * A picture that `open` shows at full size: in the lightbox of the gallery
 * around it, which steps on to the gallery's other pictures, or else in one
 * of its own, `lightbox`, for the caller to render. `ref` goes on what shows
 * the picture on the page, which gives its place in the gallery. Without an
 * `item`, there is nothing to show.
 */
export function useLightbox<E extends Element>(item: LightboxItem | undefined) {
  const gallery = useContext(Pictures);
  const latest = useRef(item);
  latest.current = item;
  const at = useRef<E>(null);
  const [alone, setAlone] = useState(false);
  const ref = useCallback(
    (el: E | null) => {
      if (!el) return;
      at.current = el;
      const remove = gallery?.add(el, () => latest.current);
      return () => {
        at.current = null;
        remove?.();
      };
    },
    [gallery],
  );
  const open = () => {
    if (gallery && at.current) gallery.open(at.current);
    else if (latest.current) setAlone(true);
  };
  const lightbox = alone && item ? <Lightbox items={[item]} onClose={() => setAlone(false)} /> : null;
  return { ref, open, lightbox };
}
