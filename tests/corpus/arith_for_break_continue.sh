# mode: bash
for ((i = 0; i < 6; i++)); do
    if ((i == 2)); then
        continue
    fi
    if ((i == 4)); then
        break
    fi
    echo "i=$i"
done
echo "after=$i status=$?"
for ((a = 0; a < 3; a++)); do
    for ((b = 0; b < 3; b++)); do
        if ((b == 1)); then
            break 2
        fi
        echo "$a.$b"
    done
done
echo "done=$?"
