/** A delivery day, as the platform's slots API gives it. */
export type Slot = {
  /** `2026-10-29` */
  date: string;
  /** `Thu 29 Oct`, in the shop's locale */
  label: string;
  full: boolean;
};
