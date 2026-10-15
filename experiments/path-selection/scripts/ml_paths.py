"""The unique leaf paths of a JSON document, in the notation shrinkfit uses."""


def paths(o, p="", out=None):
    """All unique leaf paths; arrays wildcarded (/a/*/b); same format as shrinkfit's candidates.

    Returns a dict mapping each path to the number of scalar values found there
    (0 for an empty container)."""
    out = {} if out is None else out
    if isinstance(o, dict):
        if not o and p:
            out.setdefault(p, 0)
        for k, v in o.items():
            paths(v, p + "/" + k, out)
    elif isinstance(o, list):
        if not o and p:
            out.setdefault(p, 0)
        for v in o:
            paths(v, p + "/*", out)
    else:
        out[p or "/"] = out.get(p or "/", 0) + 1
    return out
