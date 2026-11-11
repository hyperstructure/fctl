// A section of a session's details rail (and of the details sheet on a phone): a small heading,
// with what sums it up at its end, over its content. Sections are divided by space and their
// headings, not by lines.

export function RailSection({
  title,
  meta,
  children,
}: {
  title: string;
  /** What sums it up, at the heading's end: "9 of 15". */
  meta?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="px-5 py-4">
      <div className="mb-2.5 flex items-baseline justify-between gap-3">
        <h3 className="text-[12.5px] font-medium text-ink-3">{title}</h3>
        {meta && <span className="shrink-0 truncate text-[11.5px] text-ink-4 tabular-nums">{meta}</span>}
      </div>
      {children}
    </section>
  );
}
