pub(in crate::tex) fn fixture() -> Vec<u8> {
    let streams = ["1 0 0 rg 0 150 100 50 re f", "0 0 1 rg 0 0 50 100 re f"];
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] /Contents 4 0 R /Resources << >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", streams[0].len(), streams[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 100] /Rotate 90 /Contents 6 0 R /Resources << >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", streams[1].len(), streams[1]),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = vec![0];
    for (ix, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", ix + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()));
    for offset in &offsets[1..] {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len()
    ));
    pdf.into_bytes()
}

#[test]
fn renders_pages_with_correct_orientation_color_and_rotation() {
    let pdf = fixture();
    assert_eq!(super::pages(&pdf).unwrap(), 2);
    let page = super::render(&pdf, 0).unwrap();
    assert_eq!((page.width, page.height), (200, 400));
    let bytes = page.image.as_bytes(0).unwrap();
    let at =
        |x, y| &bytes[((y * page.width + x) * 4) as usize..((y * page.width + x) * 4 + 4) as usize];
    assert_eq!(at(10, 10), &[0, 0, 255, 255]);
    assert_eq!(at(10, page.height - 10), &[255, 255, 255, 255]);
    let rotated = super::render(&pdf, 1).unwrap();
    assert_eq!((rotated.width, rotated.height), (200, 100));
    assert_eq!(&rotated.image.as_bytes(0).unwrap()[0..4], &[255, 0, 0, 255]);
}

#[test]
fn rejects_invalid_documents_and_pages() {
    assert!(super::pages(b"not a PDF").is_err());
    assert!(super::render(&fixture(), 2).is_err());
    assert!(super::render(&fixture(), usize::MAX).is_err());
}
