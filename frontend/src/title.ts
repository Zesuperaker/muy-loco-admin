import { useEffect } from "react";

/** Show `title` as the page's title while the component is mounted. */
export function useTitle(title: string) {
  useEffect(() => {
    document.title = title;
  }, [title]);
}
