# mode: bash
# The point of brace expansion: one word in the script, several arguments to the command.
mkdir -p sub/{one,two}
ls sub
for f in {1..3}; do
  printf '%s ' "$f"
done
echo
touch file{1,2}.txt
ls file*.txt
