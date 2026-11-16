#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
// A scheduling gate only: pause this review's down process immediately before its selected unlink.
static void gate(const char *path) {
 const char *target=getenv("AGORA_REVIEW_LOCK");
 const char *dir=getenv("AGORA_REVIEW_GATE");
 static int used=0;
 if(!target||!dir||strcmp(path,target)||__sync_lock_test_and_set(&used,1)) return;
 char ready[1024],release[1024];
 snprintf(ready,sizeof(ready),"%s/ready",dir);
 snprintf(release,sizeof(release),"%s/release",dir);
 int fd=open(ready,O_WRONLY|O_CREAT|O_EXCL,0600);if(fd>=0)close(fd);
 for(int i=0;i<10000&&access(release,F_OK);i++)usleep(1000);
}
int unlink(const char *path) {
 int (*next)(const char*)=dlsym(RTLD_NEXT,"unlink");
 gate(path);return next(path);
}
int unlinkat(int fd,const char *path,int flags) {
 int (*next)(int,const char*,int)=dlsym(RTLD_NEXT,"unlinkat");
 gate(path);return next(fd,path,flags);
}

#include <sys/file.h>
// Signal that the real replacement claimant reached its OS-lock attempt.
int flock(int fd,int operation) {
 int (*next)(int,int)=dlsym(RTLD_NEXT,"flock");
 const char *marker=getenv("AGORA_REVIEW_FLOCK_READY");
 const char *target=getenv("AGORA_REVIEW_LOCK");
 if(marker&&target&&(operation&LOCK_EX)) {
  char link[64],path[1024],wanted[1024];
  snprintf(link,sizeof(link),"/proc/self/fd/%d",fd);
  ssize_t n=readlink(link,path,sizeof(path)-1);
  if(n>=0) {path[n]=0;snprintf(wanted,sizeof(wanted),"%s.guard",target);
   if(!strcmp(path,wanted)){int out=open(marker,O_WRONLY|O_CREAT|O_EXCL,0600);if(out>=0)close(out);}
  }
 }
 return next(fd,operation);
}
