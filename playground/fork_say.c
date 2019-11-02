#include <stdio.h>
#include <unistd.h>

int main() {
    if (fork()) {
        return 0;
    }

    for (int i = 0; i < 10; i++) {
        printf("i'm saying %d\n", i);
        fflush(stdout);
        sleep(1);
        if (i != 9) {
            sleep(1);
        }
    }
    return 0;
}

