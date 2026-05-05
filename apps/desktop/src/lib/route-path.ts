export function normalizeAppPathname(pathname: string): string {
  if (!pathname || pathname === "/") return "/";

  const normalized = pathname.endsWith("/")
    ? pathname.replace(/\/+$/, "") || "/"
    : pathname;

  return normalized === "/index.html" ? "/" : normalized;
}

export function isMainAppRoute(pathname: string): boolean {
  return normalizeAppPathname(pathname) === "/";
}
