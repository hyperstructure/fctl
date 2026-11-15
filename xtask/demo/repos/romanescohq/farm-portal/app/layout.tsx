import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Farm portal",
  description: "Where Romanesco's farms post next week's harvest",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body className="min-h-dvh bg-stone-50 text-stone-900 antialiased">{children}</body>
    </html>
  );
}
