# bytes SIZE KEY: SIZE bytes that look random to rustic's compression and deduplication, the same for the same KEY,
# so that a retake backs up the same files and draws the same sizes.
bytes() { head -c "$1" /dev/zero | openssl enc -aes-128-ctr -nosalt -pbkdf2 -pass "pass:$2"; }
