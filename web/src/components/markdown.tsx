// Markdown → React, without innerHTML (agent output cannot inject markup).
// Code blocks are highlighted lazily.

import { lexer, type Token, type Tokens } from "marked";
import { createContext, memo, type ReactNode, useContext, useEffect, useState } from "react";
import { CopyButton, Lightbox } from "@/components/ui";
import { cn } from "@/lib/cn";
import { highlight } from "@/lib/highlight";

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

/** An image in the agent's answer, which a click shows at full size. */
function ZoomImage({ src, alt }: { src: string; alt: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`View ${alt || "image"}`}
        className="max-w-full cursor-zoom-in"
      >
        <Picture src={src} alt={alt} />
      </button>
      {open && <Lightbox file={{ name: alt || "Image" }} src={src} onClose={() => setOpen(false)} />}
    </>
  );
}

/** `linked`: inside a link, which a click follows. */
function inline(tokens: Token[] | undefined, key = "i", linked = false): ReactNode[] {
  if (!tokens) return [];
  return tokens.map((t, i) => {
    const k = `${key}${i}`;
    switch (t.type) {
      case "strong":
        return <strong key={k}>{inline((t as Tokens.Strong).tokens, k, linked)}</strong>;
      case "em":
        return <em key={k}>{inline((t as Tokens.Em).tokens, k, linked)}</em>;
      case "del":
        return <del key={k}>{inline((t as Tokens.Del).tokens, k, linked)}</del>;
      case "codespan":
        return <code key={k}>{(t as Tokens.Codespan).text}</code>;
      case "br":
        return <br key={k} />;
      case "link": {
        const l = t as Tokens.Link;
        const safe = /^(https?:|mailto:|#|\/)/i.test(l.href);
        return safe ? (
          <a
            key={k}
            href={l.href}
            target={l.href.startsWith("http") ? "_blank" : undefined}
            rel="noreferrer noopener"
          >
            {inline(l.tokens, k, true)}
          </a>
        ) : (
          <span key={k}>{l.text}</span>
        );
      }
      case "image": {
        const im = t as Tokens.Image;
        if (!/^https:/i.test(im.href)) return <span key={k}>{im.text}</span>;
        return linked ? (
          <Picture key={k} src={im.href} alt={im.text} />
        ) : (
          <ZoomImage key={k} src={im.href} alt={im.text} />
        );
      }
      case "text": {
        const tx = t as Tokens.Text;
        return tx.tokens ? (
          <span key={k}>{inline(tx.tokens, k, linked)}</span>
        ) : (
          <span key={k}>{decode(tx.text)}</span>
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
        <span className="absolute top-2 right-3 font-sans text-[10.5px] tracking-wide text-ink-4 uppercase group-hover/code:opacity-0">
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
                  <th key={j}>{inline(h.tokens, `${k}h${j}`)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tb.rows.map((r, j) => (
                <tr key={j}>
                  {r.map((c, n) => (
                    <td key={n}>{inline(c.tokens, `${k}r${j}${n}`)}</td>
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
