import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { gravatarNews, gravatarPicture } from "./gravatar";

const me = (avatar_url?: string, avatar_source?: G.AvatarSource): G.Me =>
  ({
    user: { id: "usr_1", name: "Ada Lovelace", email: "ada@example.com", avatar_url },
    avatar_source,
    gravatar: true,
  }) as G.Me;

function memory() {
  const kept = new Map<string, string>();
  return {
    getItem: (k: string) => kept.get(k) ?? null,
    setItem: (k: string, v: string) => void kept.set(k, v),
  };
}

describe("gravatarPicture", () => {
  it("is the picture only when it is their Gravatar", () => {
    expect(gravatarPicture(me("/api/v1/users/usr_1/avatar?v=1", "gravatar"))).toBe(
      "/api/v1/users/usr_1/avatar?v=1",
    );
    expect(gravatarPicture(me("/api/v1/users/usr_1/avatar?v=1", "upload"))).toBeUndefined();
    expect(gravatarPicture(me("/api/v1/users/usr_1/avatar?v=1", "google"))).toBeUndefined();
    expect(gravatarPicture(me("/api/v1/users/usr_1/avatar?v=1"))).toBeUndefined();
    expect(gravatarPicture(me(undefined, "gravatar"))).toBeUndefined();
    expect(gravatarPicture(undefined)).toBeUndefined();
  });
});

describe("gravatarNews", () => {
  it("says so once for each Gravatar picture", () => {
    const storage = memory();
    expect(gravatarNews("/api/v1/users/usr_1/avatar?v=1", storage)).toBe(true);
    expect(gravatarNews("/api/v1/users/usr_1/avatar?v=1", storage)).toBe(false);
    // A new one from Gravatar has a new address.
    expect(gravatarNews("/api/v1/users/usr_1/avatar?v=2", storage)).toBe(true);
    expect(gravatarNews(undefined, storage)).toBe(false);
  });

  it("still says so without storage", () => {
    const broken = {
      getItem: () => {
        throw new Error("no storage");
      },
      setItem: () => {},
    };
    expect(gravatarNews("/api/v1/users/usr_1/avatar?v=1", broken)).toBe(true);
  });
});
