# Prints the definition of the function named NAME (a line starting with something that ends in
# NAME followed by "("), through its closing brace at column 0. Used to lift upstream functions
# verbatim into the reference harness.
BEGIN { inside = 0 }
{
    if (!inside) {
        line = $0
        if (line ~ "^[A-Za-z].*[ :*&]" NAME "\\(" && line !~ /;[ \t]*$/) { inside = 1 }
    }
    if (inside) {
        print
        if ($0 ~ /^}/) { inside = 0; print "" }
    }
}
