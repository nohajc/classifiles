# classifiles

Sort files according to their type (based on content) and append appropriate file extensions if necessary.
Useful when going through restored files with missing metadata.

## Usage

### scan directory recursively and create sorted view
```classifiles scan INPUT_DIR OUTPUT_DIR```

The OUTPUT\_DIR is populated with a directory tree based on guessed mime types and symbolic links to the original input files.

### backup sorted view
```classifiles backup INPUT_DIR OUTPUT_DIR```

Used to convert unix symbolic links to regular text files containing original file paths.

### restore sorted view
```classifiles restore INPUT_DIR OUTPUT_DIR```

Reverse of the previous operation. The backup and restore feature can be useful for storage on filesystems such as FAT32.
