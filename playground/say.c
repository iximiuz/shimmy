#include <stdio.h>
#include <unistd.h>

int main() {
    for (int i = 0; i < 10; i++) {
        printf("i'm saying %d\n", i);
        fflush(stdout);
        if (i != 9) {
            sleep(1);
        }
    }
    return 0;
}

