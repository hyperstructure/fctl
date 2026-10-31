// Markdown → React, without innerHTML (agent output cannot inject markup).
// Code blocks are highlighted lazily.

import { lexer, type Token, type Tokens } from "marked";
import { createContext, memo, type ReactNode, useContext, useEffect, useState } from "react";
import { CopyButton, useLightbox } from "@/components/ui";
import { cn } from "@/lib/cn";
import { highlight } from "@/lib/highlight";
import { textLink } from "@/lib/paths";
import { isPublishedLink } from "@/lib/published";

/**
 * The pixel sizes of images the page knows before they load, by address:
 * the screenshots a session's agent published, which its answers show.
 */
export const ImageSizes = createContext<ReadonlyMap<string, { width?: number; height?: number }>>(new Map());

/**
 * An image in the agent's answer. One of a known size takes its room at once
 * and loads as the reader nears it; any other loads with the page, so that it
 * doesn't push the page when it arrives as the reader scrolls up to it.
 */
function Picture({ src, alt }: { src: string; alt: string }) {
  const size = useContext(ImageSizes).get(src);
  const room =
    size?.width && size.height ? { width: size.width, height: size.height, loading: "lazy" as const } : {};
  return <img src={src} alt={alt} {...room} decoding="async" className="max-w-full rounded-md" />;
}

/**
 * Whether images show as pictures. Text from elsewhere (GitHub's comments) shows them as links:
 * the app loads pictures only from where it trusts, and one from anywhere else could tell who
 * reads the page.
 */
export const Pictures = createContext(true);

/**
 * Whether a picture may load from `src`: from anywhere the app's Content-Security-Policy lets it, but from the app's
 * own address only a published screenshot. The browser sends the reader's cookies there, so any other address of the
 * app's would act for them as they read: a picture can't run their export or use up their sign-in codes.
 */
function loads(src: string, origin: string = globalThis.location?.origin ?? "http://localhost"): boolean {
  try {
    return new URL(src).origin !== origin || isPublishedLink(src, origin);
  } catch {
    return false;
  }
}

/** An image as a link to it, where pictures don't show: a link like any other, never to the app's sign-in pages. */
function ImageLink({ src, alt }: { src: string; alt: string }) {
  const to = textLink(src);
  if (to?.kind !== "web") return <span>{alt || "Image"}</span>;
  return (
    <a href={to.href} target="_blank" rel="noreferrer noopener">
      {alt || "Image"}
    </a>
  );
}

/** An image in the agent's answer, which a click shows at full size. */
function ZoomImage({ src, alt }: { src: string; alt: string }) {
  const { ref, open, lightbox } = useLightbox<HTMLButtonElement>({ file: { name: alt || "Image" }, src });
  return (
    <>
      <button
        ref={ref}
        type="button"
        onClick={open}
        aria-label={`View ${alt || "image"}`}
        className="max-w-full cursor-zoom-in"
      >
        <Picture src={src} alt={alt} />
      </button>
      {lightbox}
    </>
  );
}

/** An image: a picture, which a click shows at full size unless it is in a link, or a link to it. */
function Image({ src, alt, linked }: { src: string; alt: string; linked: boolean }) {
  if (!useContext(Pictures))
    return linked ? <span>{alt || "Image"}</span> : <ImageLink src={src} alt={alt} />;
  if (!loads(src)) return <span>{alt || "Image"}</span>;
  return linked ? <Picture src={src} alt={alt} /> : <ZoomImage src={src} alt={alt} />;
}

/**
 * `s` with a place to break the line after each slash (but within `//`), where a path or URL in a
 * table's cell wraps: the cell keeps its words whole (app.css), and a path has no other place to
 * break but its hyphens.
 */
function slashes(s: string, key: string, cell: boolean): ReactNode {
  if (!cell) return s;
  const parts = s.split(/(?<=\/)(?!\/)/).filter(Boolean);
  if (parts.length < 2) return s;
  return parts.flatMap((p, j) => (j ? [<wbr key={`${key}w${j}`} />, p] : [p]));
}

/** `linked`: inside a link, which a click follows. `cell`: in a table's cell. */
function inline(tokens: Token[] | undefined, key = "i", linked = false, cell = false): ReactNode[] {
  if (!tokens) return [];
  return tokens.map((t, i) => {
    const k = `${key}${i}`;
    switch (t.type) {
      case "strong":
        return <strong key={k}>{inline((t as Tokens.Strong).tokens, k, linked, cell)}</strong>;
      case "em":
        return <em key={k}>{inline((t as Tokens.Em).tokens, k, linked, cell)}</em>;
      case "del":
        return <del key={k}>{inline((t as Tokens.Del).tokens, k, linked, cell)}</del>;
      case "codespan":
        return <code key={k}>{slashes((t as Tokens.Codespan).text, k, cell)}</code>;
      case "br":
        return <br key={k} />;
      case "link": {
        const l = t as Tokens.Link;
        const to = textLink(l.href);
        return to ? (
          <a
            key={k}
            href={to.href}
            target={to.kind === "web" ? "_blank" : undefined}
            rel="noreferrer noopener"
          >
            {inline(l.tokens, k, true, cell)}
          </a>
        ) : (
          <span key={k}>{slashes(l.text, k, cell)}</span>
        );
      }
      case "image": {
        const im = t as Tokens.Image;
        if (!/^https:/i.test(im.href)) return <span key={k}>{im.text}</span>;
        return <Image key={k} src={im.href} alt={im.text} linked={linked} />;
      }
      case "text": {
        const tx = t as Tokens.Text;
        return tx.tokens ? (
          <span key={k}>{inline(tx.tokens, k, linked, cell)}</span>
        ) : (
          <span key={k}>{slashes(decode(tx.text), k, cell)}</span>
        );
      }
      case "escape":
        return <span key={k}>{(t as Tokens.Escape).text}</span>;
      default:
        return <span key={k}>{"raw" in t ? decode(String(t.raw)) : ""}</span>;
    }
  });
}

/** marked HTML-escapes text; we render text nodes, so undo it. */
function decode(s: string): string {
  return s.replace(
    /&(amp|lt|gt|quot|#39);/g,
    (_, e) => ({ amp: "&", lt: "<", gt: ">", quot: '"', "#39": "'" })[e as string] ?? _,
  );
}

export const CodeBlock = memo(function CodeBlock({
  code,
  lang,
  className,
}: {
  code: string;
  lang: string;
  className?: string;
}) {
  const [html, setHtml] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const t = setTimeout(() => {
      highlight(code, lang).then(
        (h) => alive && setHtml(h),
        () => {},
      );
    }, 60);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [code, lang]);
  return (
    <div className={cn("group/code relative", className)}>
      <div className="absolute top-1.5 right-1.5 opacity-0 transition-opacity group-hover/code:opacity-100">
        <CopyButton text={code} />
      </div>
      {lang && lang !== "text" && (
        // Not the code's text: finding in the transcript passes over it.
        <span
          data-find-skip=""
          className="absolute top-2 right-3 font-sans text-[10.5px] tracking-wide text-ink-4 uppercase group-hover/code:opacity-0"
        >
          {lang}
        </span>
      )}
      {html ? (
        // Shiki escapes the code; the HTML is spans with color variables.
        <div
          className="[&_pre]:m-0 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-line [&_pre]:bg-sunken [&_pre]:px-4 [&_pre]:py-3 [&_pre]:font-mono [&_pre]:text-[12.75px] [&_pre]:leading-[1.65]"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="m-0 overflow-x-auto rounded-md border border-line bg-sunken px-4 py-3 font-mono text-[12.75px] leading-[1.65] text-ink-2">
          {code}
        </pre>
      )}
    </div>
  );
});

function block(t: Token, i: number): ReactNode {
  const k = `b${i}`;
  switch (t.type) {
    case "heading": {
      const h = t as Tokens.Heading;
      const Tag = (["h1", "h2", "h3", "h4", "h4", "h4"] as const)[Math.min(h.depth - 1, 5)] ?? "h4";
      return <Tag key={k}>{inline(h.tokens, k)}</Tag>;
    }
    case "paragraph":
      return <p key={k}>{inline((t as Tokens.Paragraph).tokens, k)}</p>;
    case "code": {
      const c = t as Tokens.Code;
      return <CodeBlock key={k} code={c.text} lang={c.lang?.split(/\s/)[0] ?? ""} />;
    }
    case "blockquote":
      return <blockquote key={k}>{(t as Tokens.Blockquote).tokens.map(block)}</blockquote>;
    case "list": {
      const l = t as Tokens.List;
      const items = l.items.map((it, j) => (
        <li key={`${k}${j}`} className={it.task ? "list-none" : undefined}>
          {it.task && (
            <input
              type="checkbox"
              checked={!!it.checked}
              readOnly
              className="mr-2 -ml-5 align-middle accent-[var(--c-accent)]"
            />
          )}
          {it.tokens.map((x, n) =>
            x.type === "text" ? (
              <span key={n}>{inline((x as Tokens.Text).tokens ?? [x], `${k}${j}${n}`)}</span>
            ) : (
              block(x, n)
            ),
          )}
        </li>
      ));
      return l.ordered ? (
        <ol key={k} start={typeof l.start === "number" ? l.start : undefined}>
          {items}
        </ol>
      ) : (
        <ul key={k}>{items}</ul>
      );
    }
    case "table": {
      const tb = t as Tokens.Table;
      return (
        <div key={k} className="overflow-x-auto">
          <table>
            <thead>
              <tr>
                {tb.header.map((h, j) => (
                  <th key={j}>{inline(h.tokens, `${k}h${j}`, false, true)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tb.rows.map((r, j) => (
                <tr key={j}>
                  {r.map((c, n) => (
                    <td key={n}>{inline(c.tokens, `${k}r${j}${n}`, false, true)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    case "hr":
      return <hr key={k} />;
    case "space":
      return null;
    case "html":
      return <p key={k}>{(t as Tokens.HTML).text}</p>;
    case "text": {
      const tx = t as Tokens.Text;
      return <p key={k}>{tx.tokens ? inline(tx.tokens, k) : decode(tx.text)}</p>;
    }
    default:
      return null;
  }
}

export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  let tokens: Token[];
  try {
    tokens = lexer(text, { gfm: true, breaks: false });
  } catch {
    return <p className={className}>{text}</p>;
  }
  return <div className={cn("prose-fctl", className)}>{tokens.map(block)}</div>;
});
