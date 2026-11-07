import {
  emptySubscription,
  type ProgramSubscription,
} from "@pairshaped/hypertea/program";
import type { Model, Msg } from "./app.ts";

/** Whether this key press closes the full-screen picture. Not while an input method is composing text. */
export function closesLightbox(key: {
  key: string;
  isComposing?: boolean;
}): boolean {
  return key.key === "Escape" && !key.isComposing;
}

export function lightboxKey(key: {
  key: string;
  isComposing?: boolean;
}): Msg | null {
  if (key.isComposing) return null;
  if (closesLightbox(key)) return { type: "closeImage" };
  if (key.key === "ArrowLeft") return { type: "stepImage", direction: -1 };
  if (key.key === "ArrowRight") return { type: "stepImage", direction: 1 };
  return null;
}

/** Escape closes the full-screen picture, for as long as one is open. */
export function lightboxSubscription(model: Model): ProgramSubscription<Msg> {
  if (!model.lightbox) return emptySubscription<Msg>("slot:lightbox");
  return {
    key: "lightbox",
    subscribe: (dispatch) => {
      const press = (event: KeyboardEvent) => {
        const message = lightboxKey(event);
        if (!message) return;
        event.preventDefault();
        dispatch(message);
      };
      window.addEventListener("keydown", press);
      return () => window.removeEventListener("keydown", press);
    },
  };
}
